#!/usr/bin/env bash
# Create/update the Pocket ID client and API grant used by the native practice app.
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
POCKET_ID_URL="${PRACTICE_POCKET_ID_URL:-https://auth.aries.timdumol.com}"
API_RESOURCE="${PRACTICE_OAUTH_RESOURCE:-https://mars.timdumol.com}"
CLIENT_ID="${PRACTICE_OAUTH_CLIENT_ID:-aprendiendo-practice-mobile}"
CLIENT_NAME="${PRACTICE_OAUTH_CLIENT_NAME:-Aprendiendo Practice Android}"
CLIENT_DESCRIPTION="${PRACTICE_OAUTH_CLIENT_DESCRIPTION:-Aprendiendo native Android practice client}"
CALLBACK_URI="${PRACTICE_OAUTH_REDIRECT_URI:-aprendiendo-practice-mvp://oauth/callback}"
PERMISSION_KEY="${PRACTICE_OAUTH_PERMISSION_KEY:-learning:access}"
PERMISSION_NAME="${PRACTICE_OAUTH_PERMISSION_NAME:-Aprendiendo access}"
PERMISSION_DESCRIPTION="${PRACTICE_OAUTH_PERMISSION_DESCRIPTION:-Access to Aprendiendo learning tools}"
API_KEY_FILE="${PRACTICE_POCKET_ID_API_KEY_FILE:-$ROOT/.pocket-id-api-key}"

for command in curl jq; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 127
  }
done
[[ -r "$API_KEY_FILE" ]] || {
  printf 'Pocket ID API key file is missing or unreadable: %s\n' "$API_KEY_FILE" >&2
  exit 2
}

POCKET_ID_URL="${POCKET_ID_URL%/}"
if [[ "$POCKET_ID_URL" != "https://auth.aries.timdumol.com" ]]; then
  printf 'PRACTICE_POCKET_ID_URL must be the trusted Pocket ID endpoint: https://auth.aries.timdumol.com\n' >&2
  exit 2
fi
if [[ ! "$CLIENT_ID" =~ ^[A-Za-z0-9_-]{1,128}$ ]]; then
  printf 'PRACTICE_OAUTH_CLIENT_ID contains unsupported characters.\n' >&2
  exit 2
fi
umask 077
api_config="$(mktemp /tmp/aprendiendo-pocket-id.XXXXXX)"
api_response="$(mktemp /tmp/aprendiendo-pocket-id-response.XXXXXX.json)"
api_payload="$(mktemp /tmp/aprendiendo-pocket-id-payload.XXXXXX.json)"
api_payload_tmp="$(mktemp /tmp/aprendiendo-pocket-id-payload-tmp.XXXXXX.json)"
trap 'rm -f "$api_config" "$api_response" "$api_payload" "$api_payload_tmp"' EXIT

api_key_value="$(tr -d '\r\n' < "$API_KEY_FILE")"
[[ -n "$api_key_value" ]] || {
  printf 'Pocket ID API key file is empty: %s\n' "$API_KEY_FILE" >&2
  exit 2
}
printf 'header = "X-API-KEY: %s"\n' "$api_key_value" > "$api_config"
unset api_key_value

api_request() {
  local method="$1" endpoint_path="$2" body_file="${3:-}" http_code
  case "$endpoint_path" in
    /api/apis\?pagination%5Bpage%5D=1\&pagination%5Blimit%5D=100|/api/apis)
      ;;
    /api/apis/*/permissions)
      local endpoint_id="${endpoint_path#/api/apis/}"
      endpoint_id="${endpoint_id%/permissions}"
      [[ "$endpoint_id" =~ ^[A-Za-z0-9_-]{1,128}$ ]] || {
        printf 'refusing an unrecognized Pocket ID API path: %s\n' "$endpoint_path" >&2
        exit 2
      }
      ;;
    /api/oidc/clients/*|/api/api-access/*)
      local endpoint_id="${endpoint_path##*/}"
      [[ "$endpoint_id" =~ ^[A-Za-z0-9_-]{1,128}$ ]] || {
        printf 'refusing an unrecognized Pocket ID API path: %s\n' "$endpoint_path" >&2
        exit 2
      }
      ;;
    *)
      printf 'refusing an unrecognized Pocket ID API path: %s\n' "$endpoint_path" >&2
      exit 2
      ;;
  esac
  if [[ -n "$body_file" ]]; then
    # The base URL is fixed above and endpoint_path is restricted to the API route allowlist.
    # foxguard: ignore[bash/taint-ssrf]
    http_code="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' --request "$method" --header 'Content-Type: application/json' --data-binary "@$body_file" "$POCKET_ID_URL$endpoint_path")"
  else
    # The base URL is fixed above and endpoint_path is restricted to the API route allowlist.
    # foxguard: ignore[bash/taint-ssrf]
    http_code="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' --request "$method" "$POCKET_ID_URL$endpoint_path")"
  fi
  if [[ ! "$http_code" =~ ^2[0-9][0-9]$ ]]; then
    printf 'Pocket ID request %s %s failed (HTTP %s):\n' "$method" "$endpoint_path" "$http_code" >&2
    jq . "$api_response" >&2 2>/dev/null || cat "$api_response" >&2
    exit 1
  fi
}

api_request GET '/api/apis?pagination%5Bpage%5D=1&pagination%5Blimit%5D=100'
api_id="$(jq -r --arg resource "$API_RESOURCE" '.data[] | select(.resource == $resource) | .id' "$api_response" | head -n 1)"
if [[ -z "$api_id" ]]; then
  jq -n --arg name "Aprendiendo" --arg resource "$API_RESOURCE" '{name: $name, resource: $resource}' > "$api_payload"
  api_request POST '/api/apis' "$api_payload"
  api_id="$(jq -r '.id' "$api_response")"
  printf 'Created Pocket ID API resource: %s\n' "$API_RESOURCE"
else
  printf 'Found Pocket ID API resource: %s\n' "$API_RESOURCE"
fi

api_request GET "/api/apis/$api_id"
jq --arg key "$PERMISSION_KEY" --arg name "$PERMISSION_NAME" --arg description "$PERMISSION_DESCRIPTION" '
  .permissions as $current |
  if any($current[]; .key == $key) then $current else $current + [{key: $key, name: $name, description: $description}] end |
  {permissions: .}
' "$api_response" > "$api_payload"
api_request PUT "/api/apis/$api_id/permissions" "$api_payload"
permission_id="$(jq -r --arg key "$PERMISSION_KEY" '.permissions[] | select(.key == $key) | .id' "$api_response" | head -n 1)"
[[ -n "$permission_id" && "$permission_id" != null ]] || {
  printf 'Pocket ID did not return the %s permission ID.\n' "$PERMISSION_KEY" >&2
  exit 1
}

jq -n \
  --arg id "$CLIENT_ID" \
  --arg name "$CLIENT_NAME" \
  --arg description "$CLIENT_DESCRIPTION" \
  --arg callback "$CALLBACK_URI" \
  '{
    id: $id,
    name: $name,
    description: $description,
    callbackURLs: [$callback],
    logoutCallbackURLs: [],
    isPublic: true,
    pkceEnabled: true,
    requiresReauthentication: false,
    requiresPushedAuthorizationRequests: false,
    skipConsent: false,
    credentials: {},
    launchURL: null,
    isGroupRestricted: false
  }' > "$api_payload"

client_http="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' "$POCKET_ID_URL/api/oidc/clients/$CLIENT_ID")"
case "$client_http" in
  404)
    api_request POST '/api/oidc/clients' "$api_payload"
    printf 'Created Pocket ID client: %s\n' "$CLIENT_ID"
    ;;
  2[0-9][0-9])
    jq 'del(.id)' "$api_payload" > "$api_payload_tmp"
    mv "$api_payload_tmp" "$api_payload"
    api_request PUT "/api/oidc/clients/$CLIENT_ID" "$api_payload"
    printf 'Updated Pocket ID client: %s\n' "$CLIENT_ID"
    ;;
  *)
    printf 'Pocket ID client lookup failed (HTTP %s):\n' "$client_http" >&2
    jq . "$api_response" >&2 2>/dev/null || cat "$api_response" >&2
    exit 1
    ;;
esac

jq -n --arg permission_id "$permission_id" '{userDelegatedPermissionIds: [$permission_id], clientPermissionIds: []}' > "$api_payload"
api_request PUT "/api/api-access/$CLIENT_ID" "$api_payload"

api_request GET "/api/oidc/clients/$CLIENT_ID"
client_summary="$(jq -c '{id,name,callbackURLs,isPublic,pkceEnabled,isGroupRestricted}' "$api_response")"
api_request GET "/api/api-access/$CLIENT_ID"
access_summary="$(jq -c '{userDelegatedPermissionIds,clientPermissionIds}' "$api_response")"
printf 'Verified client: %s\n' "$client_summary"
printf 'Verified API grant: %s\n' "$access_summary"
