#!/usr/bin/env bash
# Local container acceptance test for the Aprendiendo MCP service.
#
# Spins up a throwaway Postgres with the real adapter SQL, runs the hardened
# Linux image (mirroring the compose.yaml security profile), and exercises the
# full MCP Streamable HTTP protocol under the production bearer-auth mode,
# including negative (missing/wrong token) and positive paths plus the
# idempotency replay path.
#
# Requirements: a working Docker daemon. The script resolves the repo root
# relative to itself, so it runs from anywhere (Linux server or WSL).
#
#   bash scripts/smoke_test.sh
#
# The base schema below mirrors the inspected production schema documented in
# HANDOVER.md section 4; it exists only so the test database is self-contained.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker rm -f mcp-smoke-db mcp-smoke-app >/dev/null 2>&1 || true
docker network rm mcp-smoke-net >/dev/null 2>&1 || true
sleep 2
docker network create mcp-smoke-net >/dev/null

echo "[1/8] starting postgres:18"
docker run -d --name mcp-smoke-db --network mcp-smoke-net \
  -e POSTGRES_PASSWORD=smoke -e POSTGRES_DB=spanish_learning \
  postgres:18-alpine >/dev/null

ready=0
for i in $(seq 1 60); do
  if docker exec mcp-smoke-db pg_isready -U postgres -d spanish_learning >/dev/null 2>&1; then ready=1; break; fi
  sleep 1
done
[ "$ready" = "1" ] || { echo "POSTGRES NOT READY"; docker logs mcp-smoke-db 2>&1 | tail -20; exit 1; }
sleep 1

echo "[2/8] creating base schema (mirrors HANDOVER.md section 4)"
cat > /tmp/base.sql <<'EOB'
CREATE TABLE public.sessions (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, session_date date NOT NULL, exercise_type text NOT NULL, topic text, notes text, created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE public.weaknesses (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, key text NOT NULL UNIQUE, category text NOT NULL, description text NOT NULL, target_pattern text, active boolean NOT NULL DEFAULT true, first_seen date, last_seen date);
CREATE TABLE public.attempts (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, session_id bigint NOT NULL REFERENCES public.sessions(id) ON DELETE CASCADE, attempt_no integer NOT NULL, transcript text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE (session_id, attempt_no));
CREATE TABLE public.observations (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, session_id bigint NOT NULL REFERENCES public.sessions(id) ON DELETE CASCADE, attempt_id bigint REFERENCES public.attempts(id) ON DELETE CASCADE, weakness_id bigint NOT NULL REFERENCES public.weaknesses(id) ON DELETE CASCADE, outcome text NOT NULL, produced text, correction text, notes text, created_at timestamptz NOT NULL DEFAULT now());
EOB
docker cp /tmp/base.sql mcp-smoke-db:/tmp/base.sql
docker exec mcp-smoke-db psql -U postgres -d spanish_learning -v ON_ERROR_STOP=1 -f /tmp/base.sql

echo "[3/8] applying adapter SQL + contract check"
docker cp "$ROOT/sql/neon_adapter.sql" mcp-smoke-db:/tmp/adapter.sql
docker exec mcp-smoke-db psql -U postgres -d spanish_learning -v ON_ERROR_STOP=1 -f /tmp/adapter.sql >/dev/null
docker cp "$ROOT/sql/check_contract.sql" mcp-smoke-db:/tmp/check.sql
echo -n "contract: "; docker exec mcp-smoke-db psql -U postgres -d spanish_learning -tA -f /tmp/check.sql

echo "[4/8] starting app container (hardened profile, bearer auth)"
docker run -d --name mcp-smoke-app --network mcp-smoke-net \
  --read-only --tmpfs /tmp:size=8m,mode=1777 \
  --cap-drop ALL --security-opt no-new-privileges:true \
  --memory 96m --pids-limit 256 \
  -e DATABASE_URL="postgresql://postgres:smoke@mcp-smoke-db:5432/spanish_learning" \
  -e AUTH_MODE=bearer \
  -e BEARER_TOKEN="smoke-test-token-0123456789abcdef0123456789" \
  -e BIND_ADDR=0.0.0.0:8080 \
  -e RUST_LOG=aprendiendo_mcp=info \
  -p 127.0.0.1:8080:8080 \
  aprendiendo-mcp:linux >/dev/null

up=0
for i in $(seq 1 30); do
  if curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then up=1; break; fi
  sleep 1
done
[ "$up" = "1" ] || { echo "APP NOT UP"; docker logs mcp-smoke-app 2>&1 | tail -30; exit 1; }

echo "[5/8] health / ready (public, no auth)"
echo -n "health: "; curl -s http://127.0.0.1:8080/health; echo
echo -n "ready:  "; curl -s http://127.0.0.1:8080/ready; echo

TOKEN="smoke-test-token-0123456789abcdef0123456789"
H=(-H "accept: application/json, text/event-stream" -H "content-type: application/json")
AUTH=(-H "Authorization: Bearer $TOKEN")
MCP="http://127.0.0.1:8080/mcp"

echo "[6/8] auth gate: missing / wrong token rejected, valid token accepted"
echo -n "  no token   (expect 401): "; curl -s -o /dev/null -w "%{http_code}" -X POST "$MCP" "${H[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'; echo
echo -n "  wrong token(expect 401): "; curl -s -o /dev/null -w "%{http_code}" -X POST "$MCP" "${H[@]}" -H "Authorization: Bearer not-the-token" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'; echo
echo -n "  valid token(expect 200): "; curl -s -o /dev/null -w "%{http_code}" -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"1.0"}}}'; echo

echo "[7/8] MCP initialize + tools/list (valid token)"
curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"1.0"}}}'; echo
echo -n "tools: "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | python3 -c 'import sys,json; d=json.load(sys.stdin); print(json.dumps([t["name"] for t in d["result"]["tools"]]))'

echo "[8/8] tool calls: status, context, upsert, record, replay"
echo -n "get_data_status:      "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_data_status","arguments":{}}}'; echo
echo -n "get_learning_context: "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_learning_context","arguments":{"recent_sessions":3}}}'; echo
echo -n "upsert_weakness:      "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"upsert_weakness","arguments":{"key":"ser-vs-estar","category":"grammar","description":"ser vs estar distinction","target_pattern":"ser/estar","active":true}}}'; echo
PAYLOAD='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"record_practice_session","arguments":{"idempotency_key":"smoke-key-001","session_date":"2026-08-24","exercise_type":"conversation","topic":"ser vs estar","attempts":[{"attempt_no":1,"transcript":"Yo soy cansado","observations":[{"weakness_key":"ser-vs-estar","outcome":"incorrect","produced":"soy","correction":"estoy"}]}]}}}'
echo -n "record (1st):         "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d "$PAYLOAD"; echo
echo -n "record (replay):      "; curl -s -X POST "$MCP" "${H[@]}" "${AUTH[@]}" -d "$PAYLOAD"; echo

echo "---"
echo -n "app mem: "; docker stats --no-stream --format "{{.MemUsage}} / {{.MemPerc}} (limit 96m)" mcp-smoke-app

# Leave no trace.
docker rm -f mcp-smoke-db mcp-smoke-app >/dev/null 2>&1 || true
docker network rm mcp-smoke-net >/dev/null 2>&1 || true
echo "ALL SMOKE TESTS COMPLETE"
