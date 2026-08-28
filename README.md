# Aprendiendo MCP

A small, project-scoped Model Context Protocol server for the **Aprendiendo Español** ChatGPT project. It replaces Neon project/table/schema discovery and arbitrary SQL with six stable learning operations.

For the complete implementation history, verified Neon state, deployment checklist, security notes, and remaining work, see [`HANDOVER.md`](HANDOVER.md).

## Tool surface

- `get_learning_context`
- `get_recent_practice`
- `record_practice_session`
- `get_review_queue`
- `upsert_weakness`
- `get_data_status`

The Neon connection is server configuration and is intentionally absent from every tool schema. The inspected database is a single-learner store, so no synthetic learner ID is passed through the MCP or adapter.

## Architecture

```text
ChatGPT Project
    -> HTTPS /mcp (Streamable HTTP + MCP OAuth 2.1)
    -> Aprendiendo MCP (Rust)
    -> SQLite transactions and bounded queries
    -> sessions / attempts / observations / weaknesses in /data/aprendiendo.sqlite3
```

The Rust `LearningStore` trait is the compatibility boundary, so the MCP tool schemas did not change during the move from Neon to SQLite.

## Database setup

The server creates and upgrades its SQLite tables from [`sql/sqlite_schema.sql`](sql/sqlite_schema.sql). Set:

```dotenv
DATABASE_PATH=data/aprendiendo.sqlite3
```

The migrated database retains the four original application tables and adds `recorded_requests` for idempotency. The Neon export CSV files can be re-imported with `python3 scripts/import_neon_csv.py`; this checks foreign-key integrity before committing.

## Spaced repetition

Each active weakness carries a due date, interval, ease factor, repetition count,
and lapse count. `get_review_queue` returns only due weaknesses by default;
`include_upcoming` adds future reviews after them, and `as_of` supports planning or
testing for a specific date. Recording a session updates each observed weakness once:

- `correct` schedules intervals of 1 day, 3 days, then the prior interval times
  the ease factor;
- `prompted_correct` schedules tomorrow and slightly lowers ease;
- `incorrect` or `omitted` schedules tomorrow, resets repetitions, and records a lapse.

If a session contains several observations for one weakness, its least successful
outcome controls that session's schedule. Existing databases are upgraded in
place when the server starts; existing weakness history is preserved.

## Local build and test

Rust 1.88 or newer is required.

```bash
cargo test
cargo build --release
```

Copy `.env.example` to `.env`. For local testing only:

```dotenv
AUTH_MODE=disabled
BIND_ADDR=127.0.0.1:8080
```

Then run:

```bash
cargo run --release
```

The endpoints are:

- `POST /mcp` — MCP Streamable HTTP
- `GET /health` — process liveness
- `GET /ready` — SQLite connectivity
- `GET /.well-known/oauth-protected-resource` — OAuth resource metadata

Test `/mcp` with the MCP Inspector. Initialization, all tool schemas, invalid inputs, authorization, and representative tool calls should be checked before connecting ChatGPT.

## Authentication and ChatGPT compatibility

`AUTH_MODE=bearer` implements a static bearer token for MCP clients that can
configure an `Authorization` header directly. Generate one with:

```bash
openssl rand -base64 32
```

Required settings:

- `BEARER_TOKEN`

This mode is **not compatible with ChatGPT plugin authentication** as documented
on 2026-08-26. ChatGPT expects authenticated MCP servers to implement the MCP
OAuth 2.1 authorization flow, including protected-resource metadata,
authorization-server discovery, client registration or identification, and
PKCE. ChatGPT's current connection instructions do not document a static
"paste a token" option.

`AUTH_MODE=oidc` validates signed OAuth access tokens against an identity
provider's JWKS. It requires `PUBLIC_BASE_URL`, `OIDC_ISSUER`, `OIDC_JWKS_URL`,
`OIDC_AUDIENCE`, `OIDC_ALLOWED_SUBJECT`, and `OIDC_REQUIRED_SCOPE`. Token
validation alone is not a complete OAuth 2.1 authorization server and therefore
does not yet satisfy ChatGPT's connection contract.

`AUTH_MODE=disabled` is rejected on non-loopback listeners unless `ALLOW_INSECURE_NO_AUTH=true`. That escape hatch is only for local development; do not use it on a public endpoint.

## Container deployment

```bash
docker compose build
docker compose up -d
```

Compose bind-mounts `./data` at `/data`, so the migrated database is used and
persists across container replacement. Set `APP_UID`/`APP_GID` if the files are
owned by a host user other than 1000:1000.

The example Compose service is read-only, drops Linux capabilities, and has a 96 MiB memory ceiling. Put a TLS reverse proxy in front of port 8080 and expose the stable public URL `https://your-domain.example/mcp`.

Do not deploy this service for ChatGPT until a complete MCP OAuth 2.1 flow is
implemented and the intended ChatGPT surfaces, including Android, are verified.
The static bearer mode remains useful for MCP Inspector and other clients that
support custom authorization headers.

## Important limits

- No arbitrary SQL, table listing, schema listing, Neon project management, or migrations.
- Request bodies default to 128 KiB.
- Practice payloads are capped at 64 KiB.
- Query and pool waits default to 10 seconds.
- Outputs are bounded by the database adapter functions; keep their JSON responses concise.
- Secrets and tool payloads are not logged.
