# Production deployment

This playbook deploys the Rust MCP resource server to `mars.timdumol.com` using
Docker Compose and Caddy. OAuth authorization is provided by the existing
Pocket ID deployment at `https://auth.aries.timdumol.com`. Caddy terminates TLS
and is the only container that exposes ports 80 and 443; the MCP container is
reachable only on the Compose network.

## Prerequisites

1. Ensure `mars.timdumol.com` is a proxied Cloudflare DNS record for
   `2.29.6.134`. Allow inbound TCP 80 and 443 to the origin, and set Cloudflare
   SSL/TLS encryption mode to **Full (strict)**.
2. Ensure the `timdumol` SSH account has a key from the deployment machine in
   its `~/.ssh/authorized_keys` and passwordless `sudo` access.
3. Copy `inventory/hosts.example.yml` to `inventory/hosts.yml`.
4. Copy `group_vars/mcp/secrets.example.yml` to
   `group_vars/mcp/secrets.yml`, set the Pocket ID UUID for Tim and preserve the
   embedded OAuth rollback values, then encrypt it:

   ```bash
   ansible-vault encrypt group_vars/mcp/secrets.yml
   ```

   The playbook uses the Pocket ID issuer, its advertised JWKS URI, the
   `https://mars.timdumol.com` audience/resource, and the `learning:access`
   scope. It seeds the local SQLite database only on the first deployment and
   preserves it on later runs.

Before deploying, create a distinct Pocket ID API resource with resource
`https://mars.timdumol.com`, add the `learning:access` permission, and grant
that permission as user-delegated access to a new public OIDC client. Configure
the client for PKCE S256 with this exact callback:
`https://chatgpt.com/connector_platform_oauth_redirect`. Pocket ID v2.12.0 does
not advertise DCR or CIMD on this deployment, so leave ChatGPT's Registration
URL empty and use the manually configured client details.

## Deploy

Run the repository release command from the repository root:

```bash
scripts/release.sh
```

The release script runs the tests, builds the `linux/amd64` application image
locally, gzip-compresses it while streaming it over SSH to the target, and
then runs this playbook with the preloaded image tag. Set
`APRENDIENDO_DOCKER_PLATFORM` if the target uses another architecture.

The playbook can also be run manually, but the image must already be loaded on
the target and supplied explicitly, for example:

```bash
ansible-playbook site.yml --ask-vault-pass \
  --extra-vars mcp_image=aprendiendo-mcp:release-example
```

The playbook deliberately fails before changing the server if the Pocket ID
subject or preserved rollback values remain placeholders. It keeps the existing
one-time 3072-bit RSA signing key at
`/opt/aprendiendo-mcp/secrets/oauth-signing-key.pem` for rollback; preserve this
file or embedded-mode access tokens will stop validating after a replacement.
The key is readable only by the container's fixed non-root UID (`65532`).

In OIDC mode the public checks validate health/readiness, protected-resource
metadata, Pocket ID discovery, PKCE/refresh support, and the unauthenticated
MCP OAuth challenge. They do not require Aprendiendo to expose a local
authorization-server metadata endpoint. Embedded-mode checks remain available
for rollback.

Back up Pocket ID's persistent data directory and encryption key together. On
the current host these are `/opt/fitsync/data/pocketid` and
`/opt/fitsync/secrets/pocketid_encryption_key`.
