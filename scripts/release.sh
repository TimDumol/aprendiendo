#!/usr/bin/env bash
# Build, deploy, and sanity-check Aprendiendo with concise, failure-focused output.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANSIBLE_DIR="$ROOT/deploy/ansible"
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/aprendiendo-release.XXXXXX")"
trap 'rm -rf "$LOG_DIR"' EXIT

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

for command in cargo curl python3; do
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
    VAULT_ARGS=(--ask-vault-pass)
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

echo "Aprendiendo release -> $PUBLIC_URL"
run_quietly "tests" "$LOG_DIR/tests.log" \
  cargo test --manifest-path "$ROOT/Cargo.toml" --locked --quiet
run_quietly "release build" "$LOG_DIR/build.log" \
  cargo build --manifest-path "$ROOT/Cargo.toml" --locked --release --quiet
run_quietly "deploy" "$LOG_DIR/deploy.log" \
  bash -c 'cd "$1" && shift && exec "$@"' _ "$ANSIBLE_DIR" \
  "${ANSIBLE[@]}" site.yml "${VAULT_ARGS[@]}"

curl_json() {
  curl --fail --silent --show-error \
    --retry 8 --retry-delay 2 --retry-all-errors --max-time 15 \
    "$1" -o "$2"
}

printf '%-18s' "sanity check"
if curl_json "$PUBLIC_URL/health" "$LOG_DIR/health.json" \
  && curl_json "$PUBLIC_URL/ready" "$LOG_DIR/ready.json" \
  && curl_json "$PUBLIC_URL/.well-known/oauth-protected-resource" "$LOG_DIR/resource.json" \
  && curl_json "$PUBLIC_URL/.well-known/oauth-authorization-server" "$LOG_DIR/oauth.json" \
  && python3 - "$PUBLIC_URL" "$LOG_DIR" <<'PY'
import json
import pathlib
import sys

base, directory = sys.argv[1], pathlib.Path(sys.argv[2])
load = lambda name: json.loads((directory / name).read_text())
health, ready = load("health.json"), load("ready.json")
resource, oauth = load("resource.json"), load("oauth.json")

assert health.get("status") == "ok", health
assert ready.get("status") == "ready", ready
assert resource.get("resource") == base, resource
assert base in resource.get("authorization_servers", []), resource
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
  for result in health ready resource oauth; do
    [[ -s "$LOG_DIR/$result.json" ]] && printf '%s: %s\n' "$result" "$(<"$LOG_DIR/$result.json")" >&2
  done
  exit "$status"
fi

echo "Release complete: build, deployment, and public sanity checks passed."
