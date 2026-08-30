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
  local label="$1" log="$2"
  shift 2
  printf '%-18s' "$label"
  if "$@" >"$log" 2>&1; then
    echo "ok"
  else
    local status=$?
    echo "FAILED"
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

PUBLIC_URL="${APRENDIENDO_URL:-}"
if [[ -z "$PUBLIC_URL" ]]; then
  domain="$(awk '$1 == "mcp_domain:" { print $2; exit }' "$ANSIBLE_DIR/group_vars/mcp/main.yml")"
  domain="${domain%\"}"
  domain="${domain#\"}"
  domain="${domain%\'}"
  domain="${domain#\'}"
  [[ -n "$domain" ]] || {
    echo "cannot determine mcp_domain; set APRENDIENDO_URL" >&2
    exit 2
  }
  PUBLIC_URL="https://$domain"
fi
PUBLIC_URL="${PUBLIC_URL%/}"

read_main_var() {
  local key="$1" value
  value="$(awk -v key="$key" '$1 == key ":" { sub(/^[^:]+:[[:space:]]*/, ""); print; exit }' "$ANSIBLE_DIR/group_vars/mcp/main.yml")"
  value="${value#\"}"
  value="${value%\"}"
  value="${value#\'}"
  value="${value%\'}"
  printf '%s' "$value"
}

AUTH_MODE="${APRENDIENDO_AUTH_MODE:-$(read_main_var mcp_auth_mode)}"
OIDC_ISSUER="${APRENDIENDO_OIDC_ISSUER:-$(read_main_var mcp_oidc_issuer)}"
OIDC_REQUIRED_SCOPE="${APRENDIENDO_OIDC_REQUIRED_SCOPE:-$(read_main_var mcp_oidc_required_scope)}"
OIDC_JWKS_URL="${APRENDIENDO_OIDC_JWKS_URL:-$(read_main_var mcp_oidc_jwks_url)}"

[[ -n "$AUTH_MODE" ]] || {
  echo "cannot determine mcp_auth_mode; set APRENDIENDO_AUTH_MODE" >&2
  exit 2
}

echo "Aprendiendo release -> $PUBLIC_URL"
run_quietly "tests" "$LOG_DIR/tests.log" \
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

run_quietly "bootstrap" "$LOG_DIR/bootstrap.log" \
  bash -c 'cd "$1" && shift && exec "$@"' _ "$ANSIBLE_DIR" \
  "${ANSIBLE[@]}" site.yml --tags bootstrap "${VAULT_ARGS[@]}"
run_quietly "docker build" "$LOG_DIR/docker-build.log" \
  docker build --platform "$DOCKER_PLATFORM" --tag "$IMAGE_REF" "$ROOT"

stream_image() {
  local image="$1"
  docker save "$image" \
    | gzip -1 \
    | ssh "${SSH_ARGS[@]}" "$SSH_DEST" 'gzip -d | sudo -n docker load'
}

run_quietly "image transfer" "$LOG_DIR/image-transfer.log" \
  stream_image "$IMAGE_REF"
run_quietly "deploy" "$LOG_DIR/deploy.log" \
  bash -c 'cd "$1" && shift && exec "$@"' _ "$ANSIBLE_DIR" \
  "${ANSIBLE[@]}" site.yml --extra-vars "mcp_image=$IMAGE_REF" "${VAULT_ARGS[@]}"

curl_json() {
  curl --fail --silent --show-error \
    --retry 8 --retry-delay 2 --retry-all-errors --max-time 15 \
    "$1" -o "$2"
}

check_mcp_challenge() {
  local headers="$1" body="$2" status
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
  curl_json "${OIDC_ISSUER%/}/.well-known/openid-configuration" "$LOG_DIR/oidc-discovery.json"
  local_auth_status="$(curl --silent --show-error --max-time 15 -o /dev/null -w '%{http_code}' "$PUBLIC_URL/.well-known/oauth-authorization-server")"
  [[ "$local_auth_status" == "404" ]] || {
    echo "OIDC mode must not expose local authorization-server metadata (got $local_auth_status)" >&2
    exit 1
  }
elif [[ "$AUTH_MODE" == "embedded_oauth" ]]; then
  curl_json "$PUBLIC_URL/.well-known/oauth-authorization-server" "$LOG_DIR/oauth.json"
fi

printf '%-18s' "sanity check"
if curl_json "$PUBLIC_URL/health" "$LOG_DIR/health.json" \
  && curl_json "$PUBLIC_URL/ready" "$LOG_DIR/ready.json" \
  && curl_json "$PUBLIC_URL/.well-known/oauth-protected-resource" "$LOG_DIR/resource.json" \
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
