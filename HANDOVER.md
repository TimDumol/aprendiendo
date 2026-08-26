# Aprendiendo MCP handover

Last updated: 2026-08-26 (Europe/Madrid)

## 1. Executive status

The custom MCP server is implemented in Rust and its database adapter is installed on the production `main` branch of the existing Neon project.

Current state:

- Rust server source is complete.
- The MCP exposes six Spanish-learning operations instead of Neon administration, schema-discovery, or arbitrary-SQL tools.
- The implementation was aligned to the database that actually exists, rather than the initially assumed learner-profile/review-item model.
- The `mcp_api` compatibility layer is installed on Neon `main` and its six-function contract reports `ready: true`.
- Existing learning data was not changed by the production migration.
- A dedicated database runtime role has not yet been selected or granted access.
- The HTTP service has not yet been deployed to a public HTTPS endpoint.
- A static bearer-token auth mode (`AUTH_MODE=bearer`) is implemented and verified for clients that can set an `Authorization` header directly.
- **Release is blocked pending implementation, but the authentication design is now resolved.** Current OpenAI documentation requires MCP OAuth 2.1 for authenticated ChatGPT plugins and does not document the previously assumed "paste a token" connection method. The selected replacement is an OAuth 2.1 authorization server embedded in this Rust service, with one fixed user configured by environment variables and an interactive browser login. Android availability for an unpublished personal developer connection still needs to be verified on the deployed connection.
- The Linux container has been built and smoke-tested locally (Docker in WSL): the hardened image serves the full MCP protocol against the real adapter SQL and runs at ~7 MiB RSS under the 96 MiB limit.

The next operator should begin with [Section 11](#11-next-steps-in-order).

## 2. Locations and artifacts

Project root:

```text
E:\codex\spanish\aprendiendo-mcp
```

Important files:

| File | Purpose |
| --- | --- |
| `src/main.rs` | HTTP application, MCP transport, health routes, startup and shutdown |
| `src/server.rs` | Six MCP tools, validation, tool metadata and server instructions |
| `src/db.rs` | Small Neon/Postgres connection pool and fixed adapter-function calls |
| `src/model.rs` | JSON input schemas advertised to MCP clients |
| `src/auth.rs` | OAuth bearer-token and OIDC/JWKS validation |
| `src/config.rs` | Environment configuration and safety checks |
| `sql/neon_adapter.sql` | Migration already applied to Neon `main` |
| `sql/check_contract.sql` | Read-only adapter contract verification |
| `sql/create_runtime_role.sql` | Idempotent creation + grants for the dedicated runtime role (Step 1) |
| `.env.example` | Deployment configuration template; contains no real secrets |
| `Dockerfile` | Multi-stage Linux container build |
| `compose.yaml` | Hardened, memory-limited example service |
| `tests/protocol.rs` | End-to-end MCP Streamable HTTP protocol test |
| `scripts/smoke_test.sh` | Local container acceptance test: Postgres + hardened image + full MCP protocol |
| `README.md` | Concise usage and deployment overview |

The locally built Windows executable is:

```text
E:\codex\spanish\aprendiendo-mcp\target\release\aprendiendo-mcp.exe
```

Its last verified size was 5,653,504 bytes, approximately 5.39 MiB. This executable is useful for Windows testing only. Build inside the supplied Dockerfile for the target Linux server.

## 3. Why a custom MCP was built

The generic Neon MCP is valuable for development and administration, but it exposes project discovery, branches, databases, schemas, tables, and SQL operations. Repeating that discovery in the Spanish-learning ChatGPT Project consumed time and model context.

This service makes the operational boundary much smaller:

```text
ChatGPT Project
    -> HTTPS /mcp with OAuth bearer token
    -> Aprendiendo MCP
    -> six fixed mcp_api.* Postgres functions
    -> existing public learning tables
```

The ChatGPT-facing tools contain no Neon project ID, branch ID, database name, table name, schema name, connection string, or SQL field. The server already knows its one database target.

The generic Neon MCP should remain an operator/developer tool. It should not be connected to the normal Spanish-learning chat once this MCP is deployed.

## 4. Neon environment inspected

The schema was inspected directly through the authenticated Neon MCP.

| Property | Value |
| --- | --- |
| Project name | `spanish-learning` |
| Project ID | `raspy-haze-24993627` |
| Region | `aws-us-east-2` |
| PostgreSQL | 18 |
| Production branch | `main` |
| Production branch ID | `br-damp-pine-aybt9t7q` |
| Database | `spanish_learning` |
| Storage model | Single learner |

Verified production counts immediately after migration:

| Table | Rows |
| --- | ---: |
| `public.sessions` | 11 |
| `public.attempts` | 16 |
| `public.observations` | 93 |
| `public.weaknesses` | 32 |
| Active weaknesses | 32 |

The last recorded session date at verification time was `2026-08-22`.

### Existing table model

`public.sessions`

- `id bigint` primary key
- `session_date date` required
- `exercise_type text` required
- `topic text` optional
- `notes text` optional
- `created_at timestamptz` required, default `now()`

`public.attempts`

- `id bigint` primary key
- `session_id bigint` required, references `sessions(id)` with cascade delete
- `attempt_no integer` required
- `transcript text` required
- `created_at timestamptz` required, default `now()`
- Unique constraint on `(session_id, attempt_no)`

`public.observations`

- `id bigint` primary key
- `session_id bigint` required, references `sessions(id)` with cascade delete
- `attempt_id bigint` optional, references `attempts(id)` with cascade delete
- `weakness_id bigint` required, references `weaknesses(id)` with cascade delete
- `outcome text` required
- `produced text`, `correction text`, and `notes text` optional
- `created_at timestamptz` required, default `now()`
- Allowed outcomes: `incorrect`, `correct`, `prompted_correct`, and `omitted`

`public.weaknesses`

- `id bigint` primary key
- `key text` required and unique
- `category text` required
- `description text` required
- `target_pattern text` optional
- `active boolean` required, default `true`
- `first_seen date` and `last_seen date` optional

Relationship summary:

```text
sessions 1 --- many attempts
sessions 1 --- many observations
attempts 1 --- many observations (optional link)
weaknesses 1 --- many observations
```

There is no learner-profile table and no table containing spaced-review due dates. The MCP design was changed to respect this instead of adding a parallel data model.

## 5. Database changes applied to `main`

The migration in `sql/neon_adapter.sql` was applied to the production branch in one transaction.

It added:

- Schema `mcp_api`.
- Table `mcp_api.recorded_requests`.
- Six `SECURITY DEFINER` functions listed below.
- Revocations preventing `PUBLIC` access to the new schema, functions, and table.

It did not alter, delete, or rewrite any row or definition in the four existing `public` tables.

### Idempotency table

`mcp_api.recorded_requests` contains:

- `idempotency_key text` primary key
- `session_id bigint` unique foreign key to `public.sessions(id)` with cascade delete
- `created_at timestamptz` defaulting to `now()`

`record_practice_session` takes a transaction-scoped advisory lock derived from the idempotency key. A retry returns the original session ID instead of inserting a duplicate session.

### Installed function contract

```text
mcp_api.get_learning_context(integer) -> jsonb
mcp_api.get_recent_practice(integer, text) -> jsonb
mcp_api.record_practice_session(jsonb) -> jsonb
mcp_api.get_review_queue(integer, text) -> jsonb
mcp_api.upsert_weakness(jsonb) -> jsonb
mcp_api.get_data_status() -> jsonb
```

All six signatures were verified on `main` with `sql/check_contract.sql`; the result was `ready: true`.

Every function has a fixed `search_path` and uses fully qualified table references. This matters because the service should use a pooled Neon connection and Neon pooling uses transaction-mode PgBouncer.

## 6. MCP tool surface

### `get_learning_context`

Purpose: Load compact context before teaching or generating an exercise.

Input:

- `recent_sessions`: optional integer from 1 to 20; default 5.

Output:

- Up to 12 active weaknesses ordered by incorrect count.
- A bounded summary of recent sessions with attempt and observation counts.

The hard limit of 12 weaknesses is intentional. It prevents normal chat startup from loading the entire weakness catalog.

### `get_recent_practice`

Purpose: Retrieve detailed recent sessions, attempts, and observations.

Input:

- `limit`: optional integer from 1 to 50; default 10.
- `skill`: optional filter matched against exercise type, topic, weakness key, or exact weakness category.

Output includes attempt transcripts and observation details. Transcripts are truncated at 8,000 characters by the SQL adapter. Clients should request small limits unless full history is genuinely needed.

### `record_practice_session`

Purpose: Atomically record a completed practice session.

Important input fields:

- Stable caller-generated `idempotency_key`.
- Optional `session_date` in `YYYY-MM-DD`; database date is used if absent.
- Required `exercise_type`.
- Optional `topic` and `notes`.
- Zero or more numbered attempts with transcripts.
- Attempt-level or session-level observations.
- Every observation references an existing `weakness_key`.

The operation rejects unknown weakness keys before inserting the session. Create a new key with `upsert_weakness` first.

Limits enforced in Rust:

- 100 attempts per session.
- 100 session-level observations.
- 300 total observations.
- 8,000 bytes per transcript.
- 64 KiB serialized practice payload.
- Unique positive `attempt_no` values within the request.

### `get_review_queue`

Purpose: Return a prioritized weakness-review list.

Input:

- `limit`: optional integer from 1 to 50; default 20.
- `category`: optional exact category filter.

The existing database has no due dates. Therefore this is not a spaced-repetition queue. It ranks active weaknesses by total incorrect observations and all-time error rate.

### `upsert_weakness`

Purpose: Create a weakness-catalog entry or update its metadata.

Input:

- Stable `key`.
- Required `category` and `description`.
- Optional `target_pattern`.
- Optional `active` state.

This tool replaced the original `update_learning_profile` proposal because no profile table exists.

### `get_data_status`

Purpose: Small readiness/freshness result for the database adapter.

Output:

- Adapter schema version.
- `single_learner` storage-model marker.
- Last session date.
- Counts for sessions, attempts, observations, weaknesses, and active weaknesses.

## 7. Rust service implementation

The service uses:

- Rust edition 2024, minimum declared Rust version 1.88.
- `rmcp` 3.1.4 for MCP server and Streamable HTTP support.
- Axum 0.8 for routing and middleware.
- `deadpool-postgres` with Rustls for Postgres pooling and TLS.
- Stateless/legacy-session mode disabled in the MCP transport.
- Thin LTO, size optimization, panic abort, and symbol stripping for release builds.

Default database pool size is 2 and the configuration rejects values above 8. Pool acquisition and queries default to a 10-second timeout.

HTTP endpoints:

| Endpoint | Purpose | Authentication |
| --- | --- | --- |
| `POST /mcp` | Streamable HTTP MCP | Required in OIDC mode |
| `GET /health` | Process liveness | Public |
| `GET /ready` | Database connectivity | Public |
| `GET /.well-known/oauth-protected-resource` | OAuth protected-resource metadata | Public |

The MCP server instruction explicitly tells ChatGPT not to request project IDs, database names, tables, schemas, or SQL.

## 8. Authentication and security

`AUTH_MODE=bearer` accepts a single high-entropy static bearer token (minimum 32
characters; generate with `openssl rand -base64 32`). The presented
`Authorization: Bearer` value is compared in constant time against
`BEARER_TOKEN`. This works only with MCP clients that can configure the header
directly; it is not a documented ChatGPT plugin authentication method.

The `AUTH_MODE=oidc` mode validates signed OAuth access tokens. It
verifies:

- JWT signature verification from configured JWKS.
- RS256, RS384, or RS512 only.
- Issuer validation.
- Audience validation.
- Expiry validation through the JWT library.
- Exact subject match against `OIDC_ALLOWED_SUBJECT`.
- Required whitespace-delimited OAuth scope.
- 16 KiB maximum JWT size.
- JWKS refresh on an unknown key ID, limited to once per 60 seconds.
- Ten-second HTTP timeout for JWKS retrieval.

This validation is only the resource-server half of the required MCP OAuth 2.1
flow. The project does not implement or integrate a complete authorization
server with discovery metadata, authorization and token endpoints, client
registration/identification, and PKCE. Consequently neither existing auth mode
currently provides the documented authenticated ChatGPT connection flow.

The selected replacement is a small authorization server in this same Rust
process. This is feasible for the single-learner deployment; the resource
server and authorization server may share one HTTPS origin while remaining
separate OAuth components.

This must still be a real, interactive OAuth flow. The server must not silently
"log in" on behalf of ChatGPT or place the plaintext password in configuration.
When ChatGPT opens the authorization URL, the server should display a minimal
login/authorization form. The operator enters the one configured username and
password, the server verifies the password against an Argon2id PHC hash stored
as a deployment secret, and a successful submission authorizes the requested
scope for that one user.

Planned single-user authorization-server contract:

- Use authorization code with PKCE `S256`; do not implement implicit, password,
  client-credentials, service-account, or other machine-to-machine grants.
- Publish `GET /.well-known/oauth-authorization-server` with the exact issuer,
  authorization endpoint, token endpoint, `S256`, scopes, and supported token
  endpoint authentication method.
- Implement `GET` and `POST /oauth/authorize`, `POST /oauth/token`, and a public
  JWKS endpoint for the signing key.
- Prefer ChatGPT Client ID Metadata Documents (CIMD) rather than dynamic client
  registration. Allow only the exact client metadata URL and redirect URI shown
  by ChatGPT's app-management page. Validate the fetched client metadata rather
  than accepting an arbitrary URL as a client ID.
- Echo and bind the OAuth `resource` value through authorization, code exchange,
  and the access-token audience. Bind every short-lived, single-use
  authorization code to `client_id`, `redirect_uri`, `resource`, scope, and the
  PKCE challenge.
- Issue short-lived signed access tokens with the existing expected claims:
  fixed `sub`, exact `iss` and `aud`, `exp`, `iat`, `jti`, and
  `scope=learning:access`. Add refresh-token rotation if ChatGPT requires or uses
  refresh tokens in acceptance testing; durable refresh-token state must survive
  service restarts.
- Preserve and validate OAuth `state`, add CSRF protection to the login POST,
  apply login rate limiting/backoff, return generic credential failures, and
  never log credentials, authorization codes, access tokens, or refresh tokens.
- Continue returning the protected-resource metadata document and a
  `WWW-Authenticate` challenge. Ensure all six tools expose a top-level OAuth
  `securitySchemes` declaration as required by current OpenAI documentation
  (optionally mirrored in `_meta` for compatibility), and emit
  `_meta["mcp/www_authenticate"]` from tool-level auth failures if the ChatGPT
  connection path exercises tool-level linking.

The existing OIDC/JWKS mode should remain available for a future external
identity provider. The embedded mode should load its own public verification key
directly at startup rather than trying to fetch its JWKS over HTTP from the
not-yet-listening process.

Unauthorized MCP requests receive a `WWW-Authenticate: Bearer` challenge.
`AUTH_MODE=disabled` is intended only for local testing. It is rejected on a
non-loopback listener unless `ALLOW_INSECURE_NO_AUTH=true`. Do not use that
escape hatch for a public service.

Database security design:

- MCP clients cannot submit SQL.
- Runtime queries call only fixed `mcp_api` function signatures.
- `PUBLIC` access to `mcp_api` was revoked.
- Functions are `SECURITY DEFINER`, so a restricted runtime role needs function execution but not direct access to the learning tables.
- No connection string or token is logged by application code.

## 9. Configuration

Required in every environment:

| Variable | Meaning |
| --- | --- |
| `DATABASE_URL` | Pooled Neon connection string for database `spanish_learning` |

Runtime defaults:

| Variable | Default |
| --- | --- |
| `BIND_ADDR` | `127.0.0.1:8080` |
| `DATABASE_POOL_SIZE` | `2` |
| `DATABASE_TIMEOUT_SECONDS` | `10` |
| `MAX_REQUEST_BYTES` | `131072` |
| `AUTH_MODE` | `bearer` |

Required in bearer mode:

- `BEARER_TOKEN` (minimum 32 characters)

Required in legacy OIDC mode:

- `PUBLIC_BASE_URL`
- `OIDC_ISSUER`
- `OIDC_JWKS_URL`
- `OIDC_AUDIENCE`, or omit it to use `PUBLIC_BASE_URL`
- `OIDC_ALLOWED_SUBJECT`
- `OIDC_REQUIRED_SCOPE`

Planned for embedded single-user OAuth mode (names may be finalized during
implementation):

- `PUBLIC_BASE_URL` — canonical HTTPS resource and issuer origin.
- `OAUTH_USERNAME` — the one permitted login name; storing it as a secret is
  optional.
- `OAUTH_PASSWORD_HASH` — Argon2id PHC string generated offline from the chosen
  password; never store `OAUTH_PASSWORD` or another plaintext equivalent.
- `OAUTH_SIGNING_PRIVATE_KEY` — stable RSA private key, supplied by the
  deployment secret manager; publish only the derived public key through JWKS.
- `OAUTH_SIGNING_KEY_ID` — stable `kid` for the published key.
- `OAUTH_ALLOWED_CLIENT_ID` — exact ChatGPT CIMD URL shown in app management.
- `OAUTH_REDIRECT_URI` — exact ChatGPT redirect URI shown in app management.
- `OAUTH_REQUIRED_SCOPE` — default `learning:access`.

Take care when injecting the Argon2 PHC value because it contains `$`
characters. Verify that the deployment platform passes it byte-for-byte; Docker
Compose interpolation may require escaping or a secret-file mechanism. Do not
commit the username, hash, signing key, or generated tokens.

Use a pooled hostname containing `-pooler` for `DATABASE_URL`. Use a separate owner/direct connection only for future migrations.

## 10. Validation already completed

Local verification:

- `cargo fmt --check`: passed.
- `cargo test --locked`: passed.
- Three unit tests passed.
- One Streamable HTTP integration test passed.
- `cargo clippy --all-targets --locked --offline -- -D warnings`: passed.
- `cargo build --release --locked --offline`: passed.
- The protocol test initialized the MCP, listed exactly six domain tools, confirmed no project/database/SQL fields, and successfully called `get_learning_context`.

Neon validation:

1. A temporary branch named `aprendiendo-mcp-validation-20260824` was created from `main`.
2. The complete adapter migration installed successfully there.
3. All six function signatures passed the contract check.
4. A temporary weakness, session, attempt, and observation were inserted.
5. Replaying the same idempotency key returned the original session with `recorded: false` and `idempotent_replay: true`.
6. Context, recent-practice, review-queue, and status functions returned the intended JSON structures.
7. The temporary branch was deleted, permanently removing its smoke-test data.
8. The same migration was then applied to production `main` without inserting test records.
9. Production contract verification returned `ready: true`.
10. Production base-table counts were unchanged after migration.

The temporary workspace-local Rust toolchain and Rust installer were deleted after verification. They are not recoverable locally but can be downloaded again. Source, Cargo lockfile, build outputs, and release executable remain.

Docker was not installed in the development environment, so the Docker image has not yet been built or runtime-tested. The Dockerfile currently builds with Rust 1.98 even though the package declares a minimum version of 1.88; this is intentional and acceptable.

## 11. Next steps in order

### Step 1: Create or identify the dedicated Neon runtime role

This is the most important outstanding database task. The migration intentionally left the role name as a commented placeholder.

Using an owner/direct Neon connection, create or identify a login role, then grant only the necessary capabilities. Adapt the following example rather than copying credentials into source control:

```sql
-- Create the role only if it does not already exist.
CREATE ROLE aprendiendo_mcp_runtime LOGIN PASSWORD '<generated-secret>';

GRANT CONNECT ON DATABASE spanish_learning TO aprendiendo_mcp_runtime;
GRANT USAGE ON SCHEMA mcp_api TO aprendiendo_mcp_runtime;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mcp_api TO aprendiendo_mcp_runtime;
```

Do not grant this role access to Neon administration, schema discovery, or the `public` tables. The `SECURITY DEFINER` functions are the intended boundary.

Verify the role before deployment:

```sql
SET ROLE aprendiendo_mcp_runtime;
SELECT mcp_api.get_data_status();
SELECT mcp_api.get_learning_context(1);
RESET ROLE;
```

Also verify that direct reads such as `SELECT * FROM public.sessions` fail for that role.

### Step 2: Obtain the pooled runtime connection string

Generate a pooled Neon connection string for:

- Project `spanish-learning`
- Branch `main`
- Database `spanish_learning`
- The dedicated runtime role

Store it as a deployment secret named `DATABASE_URL`. Do not commit `.env`.

### Step 3: Implement the embedded single-user OAuth server

DECIDED, NOT YET IMPLEMENTED (2026-08-26): replace the abandoned ChatGPT static
token plan with the embedded single-user flow specified in
[Section 8](#8-authentication-and-security). The fixed account reduces
user-management scope, but it does not remove any OAuth protocol requirements.

Implementation order:

1. Add an `embedded_oauth` authentication mode and configuration validation.
2. Add Argon2id password verification and locally loaded RSA signing/verification
   keys. Keep authorization codes in a bounded, expiring, single-use store;
   persist only the state that must survive restart, such as refresh-token
   rotation state if refresh tokens are supported.
3. Publish authorization-server metadata and JWKS, then implement the
   authorization and token endpoints with exact redirect/client/resource/scope
   validation and PKCE `S256`.
4. Add the browser login/authorization page, CSRF protection, secure response
   headers, and login throttling. Successful login may approve the sole
   `learning:access` scope on the same page, but the page should show the client
   and requested access and provide an explicit deny path.
5. Make the resource server validate the locally issued tokens with the same
   issuer, audience, subject, expiry, and scope rules already used by OIDC mode.
6. Update tool security metadata and authentication errors to trigger ChatGPT's
   linking UI, then cover the complete flow with protocol and negative tests.

The current official implementation contract is documented at
<https://developers.openai.com/plugins/build/auth>. Re-check it immediately
before implementation because client identification and callback details may
change. Do not expose the service publicly until this step and Step 6 pass.

### Step 4: Build and run the Linux container

On a machine with Docker:

```bash
docker compose build
docker compose up -d
```

The example Compose configuration:

- Binds only to `127.0.0.1:8080` on the host.
- Sets a 96 MiB memory limit.
- Runs a read-only filesystem.
- Drops all Linux capabilities.
- Enables `no-new-privileges`.
- Provides an 8 MiB temporary filesystem.
- Runs the application as numeric non-root user/group `65532:65532`.

Check actual memory use under representative requests before reducing the 96 MiB limit.

DONE (2026-08-24): image `aprendiendo-mcp:linux` built in WSL and validated by `bash scripts/smoke_test.sh`. The test starts Postgres 18 with the real adapter SQL, runs the image under the exact compose hardening profile (read-only filesystem, all capabilities dropped, `no-new-privileges`, 96 MiB limit, non-root `65532:65532`), and verifies `/health`, `/ready`, MCP initialize, exactly six tools, `get_data_status`, `get_learning_context`, `upsert_weakness`, `record_practice_session`, and idempotency replay. Observed RSS ~7 MiB (7% of the 96 MiB limit).

### Step 5: Add TLS and a stable public URL

Place a reverse proxy or managed ingress in front of the loopback-bound service. Expose:

```text
https://<chosen-domain>/mcp
```

The same origin, without `/mcp`, should be used for `PUBLIC_BASE_URL`, the
embedded OAuth issuer, and the access-token audience. Legacy external OIDC mode
may use a separate issuer.

Verify externally:

```text
GET  /health
GET  /ready
GET  /.well-known/oauth-protected-resource
GET  /.well-known/oauth-authorization-server
GET  /oauth/jwks
GET  /oauth/authorize
POST /oauth/authorize
POST /oauth/token
POST /mcp
```

Do not expose port 8080 directly to the internet.

### Step 6: Run deployment acceptance tests

At minimum, test:

1. `/health` returns success without touching Neon.
2. `/ready` returns success and wakes a suspended Neon compute if necessary.
3. Protected-resource and authorization-server metadata contain the exact
   public resource, issuer, endpoints, scope, client-identification method, and
   PKCE `S256` declaration.
4. Authorization rejects a wrong client ID, redirect URI, resource, scope,
   response type, or PKCE method.
5. Login rejects a wrong username/password without revealing which field was
   wrong, is rate limited, and has working CSRF and deny paths.
6. Token exchange rejects an expired or replayed code, a wrong redirect/client/
   resource binding, and a missing or wrong PKCE verifier.
7. `/mcp` rejects missing, expired, wrong-audience, wrong-subject, wrong-scope,
   and invalid-signature tokens.
8. The full ChatGPT authorization-code + PKCE flow succeeds and MCP
   initialization works with the resulting token. Verify token renewal or the
   expected relinking behavior across token expiry and service restart.
9. Exactly six tools are listed, all with the required OAuth security metadata.
10. `get_data_status` and `get_learning_context` succeed using the restricted runtime role.
11. Invalid bounds and oversized inputs are rejected.
12. On a temporary Neon branch, recording a session and replaying its idempotency key behaves correctly.
13. Application logs contain neither passwords, password hashes, signing keys,
    OAuth codes/tokens, connection strings, nor tool payloads.

Avoid the first production write until the tool schemas and write confirmation behavior have been inspected from ChatGPT.

### Step 7: Connect the custom MCP to ChatGPT

After Steps 3 through 6 are complete:

1. Add the public MCP URL ending in `/mcp` from ChatGPT developer mode.
2. Complete account linking through the OAuth flow.
3. Review the six discovered tools and keep confirmations enabled for
   `record_practice_session` and `upsert_weakness` during initial use.
4. Verify the same installed connection in both the desktop and Android apps
   before relying on it for the Aprendiendo Español Project.
5. Only then remove or disable the generic Neon MCP from that Project.

The generic Neon MCP should remain available in a separate developer/operator chat for maintenance.

### Step 8: Add operating safeguards

Recommended after initial deployment:

- Configure restart and health monitoring.
- Alert on repeated `/ready` failures and authentication failures without logging bearer tokens.
- Track request latency, database-pool wait time, tool error count, and process memory.
- Keep `DATABASE_POOL_SIZE=2` initially.
- Use stable, high-entropy idempotency keys from the client.
- Confirm Neon restore/history retention meets the desired recovery objective.
- Store deployment configuration and secrets in the server's secret manager.
- Record every future adapter change as a reviewed SQL migration and test it on a Neon branch first.

## 12. Known limitations and decisions

- The database and authorization model support exactly one learner. Multi-user use would require learner ownership columns, row-level isolation, and removing the single `OIDC_ALLOWED_SUBJECT` assumption.
- Review priority is based on all-time observation counts. There is no recency weighting, mastery decay, next-review date, or spaced-repetition scheduler.
- `get_learning_context` returns only the top 12 weaknesses. Use `get_review_queue` with a category when deeper inspection is needed.
- `get_recent_practice` can still produce a large response if called with a high limit because it includes nested transcripts. Prefer limits between 3 and 10.
- A weakness must exist before it can be referenced by a recorded observation. This is deliberate to avoid silently creating low-quality duplicate keys.
- `upsert_weakness` can replace category, description, target pattern, and active state for an existing key. Treat it as a confirmed write action.
- The SQL adapter trusts the Rust layer for most payload-size and text validation. The dedicated runtime role must therefore have only function execution, not direct arbitrary SQL access.
- Legacy external OIDC startup requires the provider JWKS endpoint to be
  reachable. Embedded OAuth mode must avoid a self-fetch startup dependency by
  deriving/loading its verification key locally.
- The embedded authorization server intentionally has no signup, password
  reset, account recovery, or multi-user management. Losing the password or its
  hash requires an operator-driven secret rotation and relinking the connection.
- The service has not been load-tested or container-tested on the final server.
- There is no metrics endpoint yet; observability currently relies on structured logs and health routes.
- The workspace was not a Git repository when implementation began, so there is no commit or pull request recording these changes.

## 13. Operational checks

Read-only database contract check:

```bash
psql "$DATABASE_URL" -f sql/check_contract.sql
```

Expected result contains:

```json
{
  "schema_version": 1,
  "ready": true
}
```

Local development mode:

```dotenv
DATABASE_URL=<pooled-Neon-URL>
AUTH_MODE=disabled
BIND_ADDR=127.0.0.1:8080
```

```bash
cargo run --release
```

Never combine `AUTH_MODE=disabled` with a publicly reachable listener.

## 14. Rollback information

No rollback has been executed. If the adapter must be removed, first stop all MCP instances. The following objects are isolated from the existing learning tables:

```text
mcp_api.get_learning_context(integer)
mcp_api.get_recent_practice(integer,text)
mcp_api.record_practice_session(jsonb)
mcp_api.get_review_queue(integer,text)
mcp_api.upsert_weakness(jsonb)
mcp_api.get_data_status()
mcp_api.recorded_requests
mcp_api schema
```

Dropping `mcp_api` with `CASCADE` would remove the functions and idempotency records but would not delete the referenced `public.sessions`, `public.attempts`, `public.observations`, or `public.weaknesses` rows. It is still a destructive operation and should be tested on a Neon branch and explicitly approved before execution.

Prefer a Neon branch or point-in-time restore for recovery from an unintended application write to the existing learning tables.

## 15. Final production-readiness checklist

- [x] Existing Neon schema inspected.
- [x] MCP contract redesigned around the real schema.
- [x] Rust server implemented.
- [x] Input bounds and idempotency implemented.
- [x] OAuth access-token validation (resource-server half) implemented.
- [x] Local tests, formatting, Clippy, and release build passed.
- [x] SQL adapter smoke-tested on a temporary Neon branch.
- [x] Temporary validation branch deleted.
- [x] SQL adapter installed on production `main`.
- [x] Production contract verified as ready.
- [ ] Dedicated Neon runtime role created and granted only function access.
- [ ] Pooled runtime connection stored as a deployment secret.
- [ ] Embedded single-user OAuth 2.1 authorization server implemented.
- [ ] Fixed login uses an offline-generated Argon2id password hash; no plaintext
  password is deployed.
- [ ] Authorization code + PKCE, CIMD client validation, signed tokens, metadata,
  JWKS, and OAuth negative tests pass.
- [x] Linux container built and tested.
- [ ] TLS reverse proxy and public URL configured.
- [ ] Deployment acceptance tests passed.
- [ ] MCP registered and linked in ChatGPT.
- [ ] Generic Neon MCP removed from the normal learning Project.
- [ ] Monitoring and recovery procedures confirmed.

Release decision (2026-08-26): **no-go for static bearer-token authentication;
proceed with the embedded single-user OAuth 2.1 design.** Release remains a
no-go until that flow passes end-to-end ChatGPT tests. Verify the installed
connection separately on desktop and Android before removing the generic Neon
MCP from the learning Project.
