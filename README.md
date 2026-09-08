# Aprendiendo MCP

A small, project-scoped Model Context Protocol server for the **Aprendiendo Español** ChatGPT project. It provides stable learning operations backed by SQLite.

The API-powered recording-and-feedback app is documented in the
[implementation handoff](docs/recording-app/README.md) and implemented in
[`apps/practice`](apps/practice/README.md). Its native foundation adds private
offline media, durable jobs, and a separately bounded practice worker; physical
device/provider validation remains tracked in [`mobile-validation.md`](docs/recording-app/mobile-validation.md).

## Tool surface

- `get_learning_context`
- `get_recent_practice`
- `get_practice_brief`
- `get_practice_preferences`
- `update_practice_preferences`
- `validate_practice_plan`
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

The migrated database retains the original application tables and adds taxonomy, evidence, immutable FSRS audit state, activity runs/stimuli, practice items/targets, and `recorded_requests` for idempotency. Activity stimuli are bounded text or references; the server never stores or fetches binary media. The migration runner validates the exact live snapshot before changing it.

### Canonical practice-session contract

`record_practice_session` accepts exactly one required enum-valued
`exercise_type_key`. The catalog is:

| Key | Display label |
| --- | --- |
| `fluency_4_3_2` | `4-3-2` |
| `production_drill` | `production drill` |
| `translation_drill` | `translation drill` |
| `dele_a2_oral_microdrill` | `DELE A2 oral microdrill` |
| `agreement_disagreement_drill` | `agreement/disagreement drill` |
| `guided_conversation` | `guided conversation` |

The smallest useful request is:

```json
{
  "idempotency_key": "session-20260905-01",
  "exercise_type_key": "translation_drill",
  "items": [{
    "item_no": 1,
    "drill_type": "translation",
    "prompt": "I am tired.",
    "response": "Estoy cansado.",
    "outcome": "correct"
  }]
}
```

Omitted `reviewed_at` uses the current time, and omitted `session_date` uses
its scheduler-derived learning day. Lists default to empty; `topic` and
`notes` are optional. Observation numbers are required and unique across the
whole request, including item, attempt, and session observations.

Successful calls return `status: "created"`. An exact retry with the same
`idempotency_key` and canonical request returns the same data with
`status: "replayed"`; changing the payload returns an idempotency conflict.
The one-time version-9 migration clears `recorded_requests` because it is
retry metadata, while retaining sessions and all learning data. Stop the MCP
service before applying that production migration so no request straddles the
ledger reset. The removed legacy request/storage field is not accepted.

## Spontaneous production

Schema 10 adds durable, versioned preferences and explicit production evidence.
Briefs resolve saved exclusions before ranking, keep target opportunities optional,
and bound short initial written rounds. Validate each prompt before delivering one
turn; follow-ups depend on the learner's response. Recording contract 2 saves valid
practice even when an ineligible review is skipped. Review decisions distinguish
independent evidence, supplied forms, unknown assistance and retries.

See [implementation and rollout notes](SPONTANEOUS_PRODUCTION_IMPLEMENTATION.md)
for the typed contracts, conservative rating policy, migration/rollback procedure,
and executable request/response examples. The approved learner seed is explicit
and idempotent; other databases retain their existing defaults. Schema 10 preserves
all historical practice, schedules, review events and existing idempotency hashes.

## Spaced repetition

Active weaknesses are the FSRS memory units. The server uses the official FSRS-6
Rust implementation with its default parameter vector and 0.90 desired retention.
`get_practice_brief` selects due/new weaknesses, recommends drill families, and
returns exact recent prompts to avoid; ChatGPT supplies the exercise language.
`record_practice_session` stores each item, target, attempt, and raw observation
atomically, then applies at most one eligible explicit rating per deliberately reviewed
weakness. New reviews require two independent, materially varied observations and
supported rating/effort evidence; valid ineligible proposals are saved with skip reasons. Incidental observations are retained without changing FSRS state.
Activity-aware briefs describe one of the ten supported activities and allocate
compatible drill items. Record each learner-facing turn separately. Spoken
answers are transcript-only; timing and hesitation data must be explicitly
reported or externally measured and are never inferred from transcript text.
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
- `POST /api/practice/v1/uploads` — authenticated durable practice media upload
- `POST /api/practice/v1/analyses` — authenticated queued delivery/coaching job

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
