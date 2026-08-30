# Delegate Aprendiendo MCP OAuth to Pocket ID

## Objective

Move OAuth authorization for Aprendiendo MCP from the embedded Rust
authorization server to the existing Pocket ID deployment:

```text
ChatGPT
  -> https://mars.timdumol.com/mcp
  -> Aprendiendo MCP (OAuth resource server)
  -> validates Pocket ID access tokens via JWKS

ChatGPT login / consent / refresh
  -> https://auth.aries.timdumol.com (Pocket ID)
```

The learning database and MCP tool behavior must remain unchanged. Pocket ID
will own login sessions, refresh tokens, revocation, and account recovery;
Aprendiendo will only validate access tokens on MCP requests.

OpenAI's current authentication guidance requires the MCP resource metadata,
authorization-server discovery, PKCE authorization-code flow, and token
validation to line up with one another. See the [OpenAI authentication
documentation](https://developers.openai.com/plugins/build/auth).

## Current state and assumptions

- `AUTH_MODE=oidc` already exists in Aprendiendo. It validates issuer,
  audience, signature, subject, expiry, and `learning:access` against a JWKS
  endpoint.
- `AUTH_MODE=embedded_oauth` is currently the production deployment and owns
  the authorization endpoints in `src/embedded_oauth.rs`.
- Pocket ID is already deployed at `https://auth.aries.timdumol.com` for
  `fitsync`, with persistent data and an encryption key on the Aries host.
- The current `fitsync` plan records Pocket ID v2.12.0 as not advertising DCR
  or Client ID Metadata Documents. Unless the running version differs, use a
  manually configured OAuth client with the exact ChatGPT callback URL.
- Aprendiendo currently has one authorized learner. This plan uses one
  allowlisted Pocket ID subject. Multi-user identity mapping is a later task.

## Target configuration contract

Use these values, with the resource/audience string kept identical everywhere:

```text
OIDC issuer:       https://auth.aries.timdumol.com
MCP public base:   https://mars.timdumol.com
MCP resource:      https://mars.timdumol.com   (current Aprendiendo shape)
Required scope:    learning:access
ChatGPT callback:  https://chatgpt.com/connector_platform_oauth_redirect
```

The current server uses `PUBLIC_BASE_URL` both for the protected-resource
metadata URL and as the default OIDC audience. Keep the root resource above
for the first migration. If Pocket ID registration requires the more specific
`https://mars.timdumol.com/mcp` resource, first introduce a separate
`OIDC_RESOURCE` setting and use it consistently for metadata and audience;
do not mix the two identifiers.

## Phase 0 — Preflight and rollback safety

- [ ] Snapshot the production Aprendiendo SQLite database and record its
  integrity result. Do not alter learning rows during this migration.
- [ ] Preserve the existing embedded OAuth password hash and RSA signing key
  until the Pocket ID cutover has passed acceptance testing. They are the
  rollback path.
- [ ] Fetch Pocket ID's public OIDC discovery document without logging tokens
  or credentials. Verify:
  - `issuer` is exactly `https://auth.aries.timdumol.com`;
  - `authorization_endpoint`, `token_endpoint`, and `jwks_uri` are present;
  - PKCE `S256` is supported;
  - the provider supports the required learning scope and offline/refresh
    access for the new client.
- [ ] Confirm the running Pocket ID version and whether DCR or CIMD is
  advertised. Prefer the current OpenAI-supported client-identification path
  when available; otherwise follow the existing `fitsync` manual-client
  pattern.
- [ ] Obtain the learner's Pocket ID UUID subject through the existing
  administrator/API process. Store it as deployment secret/configuration
  material; never put access or refresh tokens in Git, logs, or test fixtures.

## Phase 1 — Make Aprendiendo an OIDC resource server

### Configuration and code

- [ ] Add the Pocket ID production profile to `.env.example` and document the
  required values:

  ```dotenv
  AUTH_MODE=oidc
  PUBLIC_BASE_URL=https://mars.timdumol.com
  OIDC_ISSUER=https://auth.aries.timdumol.com
  OIDC_JWKS_URL=<exact jwks_uri from Pocket ID discovery>
  OIDC_AUDIENCE=https://mars.timdumol.com
  OIDC_ALLOWED_SUBJECT=<Pocket ID UUID>
  OIDC_REQUIRED_SCOPE=learning:access
  ```

- [ ] Keep `OIDC_JWKS_URL` equal to Pocket ID's advertised `jwks_uri`; do not
  guess or hard-code a Pocket ID implementation path.
- [ ] Review `src/auth.rs` against a real Pocket ID access-token shape. It must
  validate an access token, not an ID token, and must accept the scope format
  Pocket ID actually emits.
- [ ] If the resource must be `/mcp`, add `OIDC_RESOURCE` as described above
  and update protected-resource metadata, the `WWW-Authenticate` challenge,
  and JWT audience validation together.
- [ ] Keep the per-tool `oauth2` security metadata when OAuth is enabled.
  Verify its scope is `learning:access`.
- [ ] Leave the embedded OAuth routes conditionally available during the
  migration so a previous release can still be restored. They must not be
  exposed by the OIDC production configuration.

### Tests

- [ ] Add OIDC tests using a local test issuer/JWKS, not production Pocket ID.
- [ ] Cover valid token, wrong issuer, wrong audience, wrong algorithm,
  unknown signing key, missing scope, wrong subject, and expired token.
- [ ] Assert protected-resource metadata contains the exact Pocket ID issuer,
  resource identifier, and `learning:access`.
- [ ] Assert unauthenticated `/mcp` returns `401` with a
  `resource_metadata` challenge.
- [ ] Keep the embedded OAuth tests passing while rollback support remains.
- [ ] Run `cargo test --locked` and `cargo build --locked --release`.

## Phase 2 — Configure Pocket ID

Perform this interactively in Pocket ID; do not add provider credentials to
the Aprendiendo repository.

- [ ] Create a distinct Pocket ID OAuth resource/client for Aprendiendo. Do
  not reuse the `fitsync` resource identifier or its audience.
- [ ] Set the resource identifier to the exact value selected in Phase 1.
- [ ] Allow `learning:access` and offline/refresh access. If Pocket ID's
  discovery advertises OIDC scopes that ChatGPT requests automatically, enable
  the required advertised scopes for the client as well.
- [ ] Configure authorization-code flow with PKCE `S256`.
- [ ] Configure the exact callback URI shown by the ChatGPT app management
  page. For the current issuer-identification-compatible setup this is expected
  to be:

  ```text
  https://chatgpt.com/connector_platform_oauth_redirect
  ```

- [ ] If the running Pocket ID version still lacks DCR/CIMD, use its manual
  client flow and leave ChatGPT's Registration URL empty.
- [ ] Ensure the resulting access token contains, at minimum, `sub`, `iss`,
  `aud` equal to the selected resource, `exp`, and `learning:access`.
- [ ] Ensure the authorization-code token response includes a refresh token
  and that the refresh token remains valid across Pocket ID restarts.

## Phase 3 — Update deployment without losing rollback

- [ ] Add non-secret Ansible variables for the Pocket ID issuer, exact JWKS
  URI, audience/resource, and required scope.
- [ ] Add the Pocket ID subject to the encrypted Ansible secrets file, or to a
  deliberately protected deployment variable if the deployment policy treats
  the UUID as non-secret.
- [ ] Change the generated production `.env` to `AUTH_MODE=oidc` and remove
  the embedded OAuth values from the active environment.
- [ ] Keep the existing embedded OAuth key/password material and the key mount
  for one rollback window, but do not use it in OIDC mode.
- [ ] Update `scripts/release.sh` so its public checks are mode-aware during
  the transition and validate:
  - Aprendiendo health and readiness;
  - protected-resource metadata pointing to Pocket ID;
  - Pocket ID OIDC discovery;
  - no local Aprendiendo authorization-server metadata is required in OIDC
    mode, while the old embedded checks remain available for rollback mode;
  - unauthenticated MCP requests receive the expected OAuth challenge.
- [ ] Update `deploy/ansible/README.md` and `README.md` to describe Pocket ID
  as the production authorization server and remove the claim that OIDC is
  incomplete when paired with an external provider.
- [ ] Deploy with the repository's prescribed release command:

  ```bash
  scripts/release.sh
  ```

## Phase 4 — Cutover acceptance

Use a fresh ChatGPT connection or explicitly reconnect the app after the
configuration change.

- [ ] Confirm ChatGPT discovers Aprendiendo protected-resource metadata.
- [ ] Confirm the login redirect goes to Pocket ID, not Aprendiendo.
- [ ] Complete login and consent once.
- [ ] Confirm ChatGPT can list tools and call representative read and write
  tools.
- [ ] Let the access token expire in a controlled test, or use a short-lived
  test client/token policy, and confirm ChatGPT refreshes without showing the
  login form again.
- [ ] Restart the Pocket ID and Aprendiendo containers independently, then
  confirm the existing ChatGPT connection can refresh/reconnect.
- [ ] Confirm a token with the wrong audience, subject, issuer, or scope is
  rejected with `401`; do not weaken validation to make the connection work.
- [ ] Confirm all learning data counts and representative tool results match
  the pre-cutover snapshot.
- [ ] Check logs for errors without printing authorization headers, cookies,
  authorization codes, access tokens, or refresh tokens.

## Phase 5 — Remove embedded OAuth after the rollback window

Only after a successful production observation period:

- [ ] Remove `AUTH_MODE=embedded_oauth` from deployment documentation and
  production configuration paths.
- [ ] Remove embedded OAuth password and RSA-key generation/mount tasks from
  Ansible, after taking an encrypted archival copy if the rollback policy
  requires it.
- [ ] Remove `src/embedded_oauth.rs`, its routes, and dependencies used only by
  the embedded provider (`argon2`, RSA signing, cookie/login support), unless
  another supported client still needs that mode.
- [ ] Delete or update embedded-only integration tests and release checks.
- [ ] Keep the resource-server OIDC tests and the manual Pocket ID acceptance
  procedure as the supported authentication test path.
- [ ] Update the final deployment documentation with Pocket ID backup
  requirements: its persistent database/data directory and encryption key
  must be backed up together.

## Rollback

If Pocket ID discovery, token claims, or ChatGPT refresh fails:

1. Restore the previous Aprendiendo release/configuration.
2. Verify the original embedded RSA key and password hash are still present.
3. Run `scripts/release.sh` and confirm the old health and OAuth checks pass.
4. Diagnose the failed Pocket ID path using redacted discovery/token metadata;
   do not copy tokens into issue reports or logs.

Rollback must not restore or overwrite the SQLite learning database.

## Definition of done

- ChatGPT authenticates through Pocket ID once and renews access without
  recurring login prompts.
- Aprendiendo never stores ChatGPT refresh tokens or user passwords.
- The MCP server validates Pocket ID access tokens against the exact issuer,
  audience, subject, expiry, signature, and scope.
- Pocket ID state and encryption material are persistent and included in the
  backup/restore procedure.
- Production release checks, local tests, and the documented rollback path all
  pass.
