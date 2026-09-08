#!/usr/bin/env bash
# Build, deploy, and sanity-check Aprendiendo with concise, failure-focused output.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANSIBLE_DIR="$ROOT/deploy/ansible"
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/aprendiendo-release.XXXXXX")"
trap 'rm -rf "$LOG_DIR"' EXIT
ANSIBLE_LOCAL_TEMP="$LOG_DIR/ansible-tmp"
mkdir -p "$ANSIBLE_LOCAL_TEMP"
export ANSIBLE_LOCAL_TEMP

run_quietly() {
  local label="$1" log_name="$2" log
  shift 2
  case "$log_name" in
    tests|bootstrap|docker-build|image-transfer|deploy)
      ;;
    *)
      echo "unrecognized release log name: $log_name" >&2
      exit 2
      ;;
  esac
  log="$LOG_DIR/$log_name.log"
  printf '%-18s' "$label"
  if "$@" >"$log" 2>&1; then
    echo "ok"
  else
    local status=$?
    echo "FAILED"
    # log_name is restricted to the fixed names above and resolved below LOG_DIR.
    # foxguard: ignore[bash/taint-path-traversal]
    tail -n 80 "$log" >&2
    exit "$status"
  fi
}

for command in cargo curl docker gzip python3 ssh; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 127
  }
done

if command -v ansible-playbook >/dev/null; then
  ANSIBLE=(ansible-playbook)
elif command -v uvx >/dev/null; then
  ANSIBLE=(uvx --from ansible-core ansible-playbook)
else
  echo "missing ansible-playbook (install Ansible or uvx)" >&2
  exit 127
fi

[[ -f "$ANSIBLE_DIR/inventory/hosts.yml" ]] || {
  echo "missing deploy/ansible/inventory/hosts.yml" >&2
  exit 2
}
[[ -f "$ANSIBLE_DIR/group_vars/mcp/secrets.yml" ]] || {
  echo "missing deploy/ansible/group_vars/mcp/secrets.yml" >&2
  exit 2
}
[[ -f "$ROOT/data/aprendiendo.sqlite3" ]] || {
  echo "missing seed database: data/aprendiendo.sqlite3" >&2
  exit 2
}

VAULT_ARGS=()
if head -n 1 "$ANSIBLE_DIR/group_vars/mcp/secrets.yml" | grep -q '^\$ANSIBLE_VAULT;'; then
  if [[ -n "${ANSIBLE_VAULT_PASSWORD_FILE:-}" ]]; then
    VAULT_ARGS=(--vault-password-file "$ANSIBLE_VAULT_PASSWORD_FILE")
  elif [[ -t 0 ]]; then
    vault_password_file="$LOG_DIR/vault-password"
    printf 'Ansible Vault password: '
    IFS= read -r -s vault_password
    printf '\n'
    printf '%s\n' "$vault_password" >"$vault_password_file"
    chmod 600 "$vault_password_file"
    unset vault_password
    VAULT_ARGS=(--vault-password-file "$vault_password_file")
  else
    echo "encrypted secrets require ANSIBLE_VAULT_PASSWORD_FILE in non-interactive runs" >&2
    exit 2
  fi
fi

domain="$(awk '$1 == "mcp_domain:" { print $2; exit }' "$ANSIBLE_DIR/group_vars/mcp/main.yml")"
domain="${domain%\"}"
domain="${domain#\"}"
domain="${domain%\'}"
domain="${domain#\'}"
if [[ -z "$domain" || ! "$domain" =~ ^[A-Za-z0-9.-]+$ ]]; then
  echo "cannot determine a valid mcp_domain from deploy configuration" >&2
  exit 2
fi
configured_public_url="https://$domain"
PUBLIC_URL="${APRENDIENDO_URL:-}"
PUBLIC_URL="${PUBLIC_URL%/}"
if [[ -n "$PUBLIC_URL" && "$PUBLIC_URL" != "$configured_public_url" ]]; then
  echo "APRENDIENDO_URL must match the configured HTTPS deployment origin: $configured_public_url" >&2
  exit 2
fi
PUBLIC_URL="$configured_public_url"

read_main_var() {
  local key="$1" value
  value="$(awk -v key="$key" '$1 == key ":" { sub(/^[^:]+:[[:space:]]*/, ""); print; exit }' "$ANSIBLE_DIR/group_vars/mcp/main.yml")"
  value="${value#\"}"
  value="${value%\"}"
  value="${value#\'}"
  value="${value%\'}"
  printf '%s' "$value"
}

AUTH_MODE="$(read_main_var mcp_auth_mode)"
configured_oidc_issuer="$(read_main_var mcp_oidc_issuer)"
configured_oidc_required_scope="$(read_main_var mcp_oidc_required_scope)"
configured_oidc_jwks_url="$(read_main_var mcp_oidc_jwks_url)"
if [[ -n "${APRENDIENDO_AUTH_MODE:-}" && "$APRENDIENDO_AUTH_MODE" != "$AUTH_MODE" ]]; then
  echo "APRENDIENDO_AUTH_MODE must match the deploy configuration" >&2
  exit 2
fi
if [[ -n "${APRENDIENDO_OIDC_ISSUER:-}" && "$APRENDIENDO_OIDC_ISSUER" != "$configured_oidc_issuer" ]]; then
  echo "APRENDIENDO_OIDC_ISSUER must match the deploy configuration" >&2
  exit 2
fi
if [[ -n "${APRENDIENDO_OIDC_REQUIRED_SCOPE:-}" && "$APRENDIENDO_OIDC_REQUIRED_SCOPE" != "$configured_oidc_required_scope" ]]; then
  echo "APRENDIENDO_OIDC_REQUIRED_SCOPE must match the deploy configuration" >&2
  exit 2
fi
if [[ -n "${APRENDIENDO_OIDC_JWKS_URL:-}" && "$APRENDIENDO_OIDC_JWKS_URL" != "$configured_oidc_jwks_url" ]]; then
  echo "APRENDIENDO_OIDC_JWKS_URL must match the deploy configuration" >&2
  exit 2
fi
OIDC_ISSUER="$configured_oidc_issuer"
OIDC_REQUIRED_SCOPE="$configured_oidc_required_scope"
OIDC_JWKS_URL="$configured_oidc_jwks_url"

[[ -n "$AUTH_MODE" ]] || {
  echo "cannot determine mcp_auth_mode; set APRENDIENDO_AUTH_MODE" >&2
  exit 2
}

PRACTICE_ENV_FILE="${APRENDIENDO_PRACTICE_ENV_FILE:-$ROOT/apps/practice/.env.mcp}"
PRACTICE_VARS_ARGS=()
if [[ -f "$PRACTICE_ENV_FILE" ]]; then
  PRACTICE_VARS_FILE="$LOG_DIR/practice-vars.json"
  python3 - "$PRACTICE_ENV_FILE" "$PRACTICE_VARS_FILE" <<'PY'
import json
import pathlib
import sys

source, destination = map(pathlib.Path, sys.argv[1:])
values = {}
for raw in source.read_text().splitlines():
    line = raw.strip()
    if not line or line.startswith("#") or "=" not in line:
        continue
    key, value = line.split("=", 1)
    key = key.strip()
    if key not in {"GEMINI_API_KEY", "GEMINI_MODEL"}:
        continue
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        value = value[1:-1]
    values[key] = value

if not values.get("GEMINI_API_KEY"):
    raise SystemExit(f"{source} does not contain a non-empty GEMINI_API_KEY")

destination.write_text(
    json.dumps(
        {
            "mcp_gemini_api_key": values["GEMINI_API_KEY"],
            "mcp_gemini_model": values.get("GEMINI_MODEL", "gemini-3.8-flash"),
        }
    )
)
PY
  chmod 600 "$PRACTICE_VARS_FILE"
  PRACTICE_VARS_ARGS=(--extra-vars "@$PRACTICE_VARS_FILE")
fi

echo "Aprendiendo release -> $PUBLIC_URL"
run_quietly "tests" tests \
  cargo test --manifest-path "$ROOT/Cargo.toml" --locked --quiet

SSH_HOST="${APRENDIENDO_SSH_HOST:-$(awk '$1 == "ansible_host:" { print $2; exit }' "$ANSIBLE_DIR/inventory/hosts.yml")}"
SSH_USER="${APRENDIENDO_SSH_USER:-$(awk '$1 == "ansible_user:" { print $2; exit }' "$ANSIBLE_DIR/inventory/hosts.yml")}"
SSH_KEY="${APRENDIENDO_SSH_KEY:-$(awk '$1 == "ansible_ssh_private_key_file:" { print $2; exit }' "$ANSIBLE_DIR/inventory/hosts.yml")}"
SSH_PORT="${APRENDIENDO_SSH_PORT:-$(awk '$1 == "ansible_port:" { print $2; exit }' "$ANSIBLE_DIR/inventory/hosts.yml")}"
[[ -n "$SSH_HOST" ]] || {
  echo "cannot determine SSH host from inventory; set APRENDIENDO_SSH_HOST" >&2
  exit 2
}

SSH_DEST="${SSH_USER:+$SSH_USER@}$SSH_HOST"
SSH_ARGS=(-o BatchMode=yes -o ConnectTimeout=15)
[[ -n "$SSH_KEY" ]] && SSH_ARGS+=(-i "$SSH_KEY")
[[ -n "$SSH_PORT" ]] && SSH_ARGS+=(-p "$SSH_PORT")

IMAGE_NAME="${APRENDIENDO_IMAGE_NAME:-aprendiendo-mcp}"
IMAGE_TAG="${APRENDIENDO_IMAGE_TAG:-release-$(date -u +%Y%m%d%H%M%S)}"
IMAGE_REF="$IMAGE_NAME:$IMAGE_TAG"
DOCKER_PLATFORM="${APRENDIENDO_DOCKER_PLATFORM:-linux/amd64}"

run_quietly "bootstrap" bootstrap \
  bash -c 'cd "$1" && shift && exec "$@"' _ "$ANSIBLE_DIR" \
  "${ANSIBLE[@]}" site.yml --tags bootstrap "${VAULT_ARGS[@]}"
run_quietly "docker build" docker-build \
  docker build --platform "$DOCKER_PLATFORM" --tag "$IMAGE_REF" "$ROOT"

stream_image() {
  local image="$1"
  docker save "$image" \
    | gzip -1 \
    | ssh "${SSH_ARGS[@]}" "$SSH_DEST" 'gzip -d | sudo -n docker load'
}

run_quietly "image transfer" image-transfer \
  stream_image "$IMAGE_REF"
run_quietly "deploy" deploy \
  bash -c 'cd "$1" && shift && exec "$@"' _ "$ANSIBLE_DIR" \
  "${ANSIBLE[@]}" site.yml --extra-vars "mcp_image=$IMAGE_REF" \
  "${PRACTICE_VARS_ARGS[@]}" "${VAULT_ARGS[@]}"

curl_json() {
  local target="$1" output="$2" url
  case "$target" in
    health) url="$PUBLIC_URL/health" ;;
    ready) url="$PUBLIC_URL/ready" ;;
    resource) url="$PUBLIC_URL/.well-known/oauth-protected-resource" ;;
    oidc-discovery) url="${OIDC_ISSUER%/}/.well-known/openid-configuration" ;;
    oauth) url="$PUBLIC_URL/.well-known/oauth-authorization-server" ;;
    *) echo "unrecognized release URL target: $target" >&2; exit 2 ;;
  esac
  # url is selected only from the validated deployment/OIDC origins and fixed paths above.
  # foxguard: ignore[bash/taint-ssrf]
  curl --fail --silent --show-error \
    --retry 8 --retry-delay 2 --retry-all-errors --max-time 15 \
    "$url" -o "$output"
}

check_mcp_challenge() {
  local headers="$1" body="$2" status
  # PUBLIC_URL is the validated deployment origin derived from inventory above.
  # foxguard: ignore[bash/taint-ssrf]
  status="$(curl --silent --show-error --max-time 15 \
    -D "$headers" -o "$body" -w '%{http_code}' \
    -X POST "$PUBLIC_URL/mcp" \
    -H 'accept: application/json, text/event-stream' \
    -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}')"
  [[ "$status" == 401 ]] || {
    echo "expected unauthenticated /mcp request to return 401, got $status" >&2
    return 1
  }
  python3 - "$headers" "$AUTH_MODE" "$PUBLIC_URL" "$OIDC_REQUIRED_SCOPE" <<'PY'
import pathlib
import sys

headers = pathlib.Path(sys.argv[1]).read_text().lower()
mode, base, scope = sys.argv[2:]
assert "www-authenticate:" in headers, headers
if mode == "oidc":
    assert "resource_metadata=\"" + base + "/.well-known/oauth-protected-resource\"" in headers, headers
    assert "scope=\"" + scope + "\"" in headers, headers
PY
}

if [[ "$AUTH_MODE" == "oidc" ]]; then
  curl_json oidc-discovery "$LOG_DIR/oidc-discovery.json"
  local_auth_status="$(curl --silent --show-error --max-time 15 -o /dev/null -w '%{http_code}' "$PUBLIC_URL/.well-known/oauth-authorization-server")"
  [[ "$local_auth_status" == "404" ]] || {
    echo "OIDC mode must not expose local authorization-server metadata (got $local_auth_status)" >&2
    exit 1
  }
elif [[ "$AUTH_MODE" == "embedded_oauth" ]]; then
  curl_json oauth "$LOG_DIR/oauth.json"
fi

printf '%-18s' "sanity check"
if curl_json health "$LOG_DIR/health.json" \
  && curl_json ready "$LOG_DIR/ready.json" \
  && curl_json resource "$LOG_DIR/resource.json" \
  && check_mcp_challenge "$LOG_DIR/challenge.headers" "$LOG_DIR/challenge.body" \
  && python3 - "$PUBLIC_URL" "$LOG_DIR" "$AUTH_MODE" "$OIDC_ISSUER" "$OIDC_JWKS_URL" "$OIDC_REQUIRED_SCOPE" <<'PY'
import json
import pathlib
import sys

base, directory, mode, issuer, jwks_url, required_scope = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6]
load = lambda name: json.loads((directory / name).read_text())
health, ready, resource = load("health.json"), load("ready.json"), load("resource.json")

assert health.get("status") == "ok", health
assert ready.get("status") == "ready", ready
assert resource.get("resource") == base, resource
assert required_scope in resource.get("scopes_supported", []), resource

if mode == "oidc":
    assert issuer in resource.get("authorization_servers", []), resource
    discovery = load("oidc-discovery.json")
    assert discovery.get("issuer") == issuer, discovery
    assert discovery.get("authorization_endpoint"), discovery
    assert discovery.get("token_endpoint"), discovery
    assert discovery.get("jwks_uri") == jwks_url, discovery
    assert "S256" in discovery.get("code_challenge_methods_supported", []), discovery
    assert "refresh_token" in discovery.get("grant_types_supported", []), discovery
    assert "offline_access" in discovery.get("scopes_supported", []), discovery
elif mode == "embedded_oauth":
    assert base in resource.get("authorization_servers", []), resource
    oauth = load("oauth.json")
    assert oauth.get("issuer") == base, oauth
    assert oauth.get("authorization_endpoint") == f"{base}/oauth/authorize", oauth
    assert oauth.get("token_endpoint") == f"{base}/oauth/token", oauth
    assert "S256" in oauth.get("code_challenge_methods_supported", []), oauth
PY
then
  echo "ok"
else
  status=$?
  echo "FAILED"
  for result in health ready resource oidc-discovery oauth; do
    [[ -s "$LOG_DIR/$result.json" ]] && printf '%s: %s\n' "$result" "$(<"$LOG_DIR/$result.json")" >&2
  done
  exit "$status"
fi

echo "Release complete: $IMAGE_REF deployed, and public sanity checks passed."
