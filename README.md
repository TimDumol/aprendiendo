# Aprendiendo MCP

A small, project-scoped Model Context Protocol server for the **Aprendiendo Español** ChatGPT project. It provides stable learning operations backed by SQLite.

## Tool surface

- `get_learning_context`
- `get_recent_practice`
- `get_practice_brief`
- `record_practice_session`
- `get_review_queue`
- `upsert_weakness`
- `get_taxonomy`
- `upsert_concept`
- `get_data_status`


## Architecture

```text
ChatGPT Project
    -> HTTPS /mcp (Streamable HTTP + MCP OAuth 2.1)
    -> Aprendiendo MCP (Rust)
    -> validates Pocket ID access tokens via JWKS
    -> SQLite transactions and bounded queries
    -> sessions / attempts / observations / weaknesses in /data/aprendiendo.sqlite3

ChatGPT login, consent, refresh, and revocation are handled by Pocket ID at
`https://auth.aries.timdumol.com`.
```

## Database setup

The server creates and upgrades its SQLite tables from [`sql/sqlite_schema.sql`](sql/sqlite_schema.sql). Set:

```dotenv
DATABASE_PATH=data/aprendiendo.sqlite3
```

The migrated database retains the original application tables and adds taxonomy, evidence, immutable FSRS audit state, practice items/targets, and `recorded_requests` for idempotency. The migration runner validates the exact live snapshot before changing it.

## Spaced repetition

Active weaknesses are the FSRS memory units. The server uses the official FSRS-6
Rust implementation with its default parameter vector and 0.90 desired retention.
`get_practice_brief` selects due/new weaknesses, recommends drill families, and
returns exact recent prompts to avoid; ChatGPT supplies the exercise language.
`record_practice_session` stores each item, target, attempt, and raw observation
atomically, then applies at most one explicit rating per deliberately reviewed
weakness. Incidental observations are retained without changing FSRS state.
Legacy scheduler columns remain for compatibility but are no longer updated.
Existing databases are upgraded in place; legacy observations remain historical
evidence and never become inferred FSRS reviews.

To apply the checked-in migration to a database explicitly, use
`cargo run --bin migrate -- path/to/database.sqlite3`. A production path must
be run through `scripts/migrate_production.sh --confirm-production` only after
the same script has succeeded on a verified snapshot copy.

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

Production uses Pocket ID as the OAuth authorization server. Aprendiendo is an
OAuth resource server: it publishes protected-resource metadata and validates
Pocket ID access tokens for the exact issuer, audience, signature, expiry,
allowlisted subject, and `learning:access` scope. It does not store ChatGPT
refresh tokens or user passwords.

The production profile is:

```dotenv
AUTH_MODE=oidc
PUBLIC_BASE_URL=https://mars.timdumol.com
OIDC_ISSUER=https://auth.aries.timdumol.com
OIDC_JWKS_URL=https://auth.aries.timdumol.com/.well-known/jwks.json
OIDC_AUDIENCE=https://mars.timdumol.com
OIDC_ALLOWED_SUBJECT=<Pocket ID UUID for the learner>
OIDC_REQUIRED_SCOPE=learning:access
```

`OIDC_JWKS_URL` must remain equal to Pocket ID's advertised `jwks_uri`; do not
replace it with an implementation-specific guessed path. The current Pocket
ID deployment supports PKCE S256 and refresh-token grants. Its manually
configured Aprendiendo client uses the exact callback
`https://chatgpt.com/connector_platform_oauth_redirect`.

`AUTH_MODE=bearer` implements a static bearer token for MCP clients that can
configure an `Authorization` header directly. Generate one with:

```bash
openssl rand -base64 32
```

Required settings:

- `BEARER_TOKEN`

This mode is useful for MCP Inspector and other clients that support custom
authorization headers; ChatGPT production authentication uses Pocket ID.

`AUTH_MODE=embedded_oauth` remains available only as a rollback path during the
observation window. Its password hash and signing key are deliberately kept out
of the active OIDC environment.

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

## Current production data location

The currently inspected production instance is `mars.timdumol.com`. The
host-side SQLite database is:

```text
/opt/aprendiendo-mcp/app/data/aprendiendo.sqlite3
```

The MCP container bind-mounts that directory as `/data`, so its database path
is `/data/aprendiendo.sqlite3`. The Ansible variable
`mcp_deploy_dir` controls the `/opt/aprendiendo-mcp` prefix. Keep this location
in sync with the actual inventory/host; the checked-in example deployment
metadata currently names `mars.timdumol.com`.

Pocket ID backups must include its persistent data directory and encryption key
together. On the current host these are `/opt/fitsync/data/pocketid` and
`/opt/fitsync/secrets/pocketid_encryption_key`.

## Important limits

- No arbitrary SQL, table listing, schema listing, or unbounded database access.
- Request bodies default to 128 KiB.
- Practice payloads are capped at 64 KiB.
- Query and pool waits default to 10 seconds.
- Outputs are bounded by the database adapter functions; keep their JSON responses concise.
- Secrets and tool payloads are not logged.
