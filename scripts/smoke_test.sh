#!/usr/bin/env bash
# Local container acceptance test for the SQLite-backed Aprendiendo MCP service.
set -euo pipefail

NAME="mcp-smoke-app"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" \
  --read-only --tmpfs /tmp:size=8m,mode=1777 --tmpfs /data:size=16m,mode=1777 \
  --cap-drop ALL --security-opt no-new-privileges:true --memory 96m --pids-limit 256 \
  -e DATABASE_PATH=/data/aprendiendo.sqlite3 \
  -e AUTH_MODE=bearer \
  -e BEARER_TOKEN="smoke-test-token-0123456789abcdef0123456789" \
  -e BIND_ADDR=0.0.0.0:8080 -e RUST_LOG=aprendiendo_mcp=info \
  -p 127.0.0.1:8080:8080 "${APRENDIENDO_IMAGE:-aprendiendo-mcp:linux}" >/dev/null

cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT

for _ in $(seq 1 30); do
  curl -sf http://127.0.0.1:8080/health >/dev/null && break
  sleep 1
done
curl -sf http://127.0.0.1:8080/health
echo

TOKEN="smoke-test-token-0123456789abcdef0123456789"
H=(-H "accept: application/json, text/event-stream" -H "content-type: application/json")
AUTH=(-H "Authorization: Bearer $TOKEN")
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST --url 'http://127.0.0.1:8080/mcp' "${H[@]}" --data-binary '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}')" = "401" ]

call() {
  # The URL is a fixed loopback endpoint; the argument is request data only.
  # foxguard: ignore[bash/taint-ssrf]
  curl -sf -X POST --url 'http://127.0.0.1:8080/mcp' "${H[@]}" "${AUTH[@]}" --data-binary "$1"
}
call '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"1.0"}}}' >/dev/null
tools="$(call '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')"
python3 -c 'import json,sys; d=json.load(sys.stdin); tools=d["result"]["tools"]; names=[x["name"] for x in tools]; assert len(names)==12 and "get_taxonomy" in names and "upsert_concept" in names, names; record=next(x for x in tools if x["name"]=="record_practice_session"); schema=record["inputSchema"]; assert "exercise_type_key" in schema["required"] and "exercise_type" not in schema["properties"]; assert record["annotations"]["idempotentHint"] is True; print(names)' <<<"$tools"

call '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_data_status","arguments":{}}}' >/dev/null
call '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"upsert_weakness","arguments":{"key":"ser-vs-estar","category":"grammar","description":"ser vs estar distinction","target_pattern":"ser/estar","active":true,"primary_concept_key":"form.grammar.syntax.copular","target_type":"form_meaning_contrast"}}}' >/dev/null
call '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_practice_brief","arguments":{"count":1,"weakness_keys":["ser-vs-estar"]}}}' >/dev/null
payload='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"record_practice_session","arguments":{"idempotency_key":"smoke-key-001","session_date":"2026-08-29","reviewed_at":"2026-08-29T18:30:00+02:00","exercise_type_key":"translation_drill","items":[{"item_no":1,"drill_type":"translation","prompt":"I am tired.","response":"Estoy cansado.","outcome":"correct","target_weakness_keys":["ser-vs-estar"],"observations":[{"observation_no":1,"weakness_key":"ser-vs-estar","outcome":"correct","role":"targeted","assessment_phase":"cold_retrieval","hint_level":"none","evidence_strength":"controlled_production","learner_effort":"some_effort"}]}],"reviews":[{"weakness_key":"ser-vs-estar","rating":"good","retrieval_mode":"controlled_production","evidence_strength":"controlled_production","evidence_observation_nos":[1],"rating_source":"assistant_suggested","evidence":{"rating_reason":"Independent production"}}]}}}'
first="$(call "$payload")"
second="$(call "${payload/id\":6/id\":7}")"
python3 -c 'import json,sys; a=json.loads(sys.argv[1])["result"]["structuredContent"]["data"]; b=json.loads(sys.argv[2])["result"]["structuredContent"]["data"]; assert a["status"]=="created" and b["session_id"]==a["session_id"] and b["status"]=="replayed"' "$first" "$second"
changed="${payload/I am tired./I am exhausted.}"
conflict="$(call "$changed")"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d["result"]["isError"] is True and "idempotency_conflict" in d["result"]["content"][0]["text"]' "$conflict"
status="$(call '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_data_status","arguments":{}}}')"
python3 -c 'import json,sys; d=json.loads(sys.argv[1])["result"]["structuredContent"]["data"]; assert d["counts"]["sessions"]==1' "$status"

docker stats --no-stream --format '{{.MemUsage}} / {{.MemPerc}} (limit 96m)' "$NAME"
echo "ALL SMOKE TESTS COMPLETE"
