# Production deployment

This playbook deploys the Rust MCP server to `mars.timdumol.com` using Docker
Compose and Caddy. Caddy terminates TLS and is the only container that exposes
ports 80 and 443; the MCP container is reachable only on the Compose network.

## Prerequisites

1. Ensure `mars.timdumol.com` is a proxied Cloudflare DNS record for
   `2.29.6.134`. Allow inbound TCP 80 and 443 to the origin, and set Cloudflare
   SSL/TLS encryption mode to **Full (strict)**.
2. Ensure the `timdumol` SSH account has a key from the deployment machine in
   its `~/.ssh/authorized_keys` and passwordless `sudo` access.
3. Copy `inventory/hosts.example.yml` to `inventory/hosts.yml`.
4. Copy `group_vars/mcp/secrets.example.yml` to
   `group_vars/mcp/secrets.yml`, replace its placeholders, and encrypt it:

   ```bash
   ansible-vault encrypt group_vars/mcp/secrets.yml
   ```

   The playbook uses ChatGPT's stable CIMD client ID and redirect URI. It seeds
   the local SQLite database only on the first deployment and preserves it on
   later runs.

## Deploy

```bash
ansible-playbook site.yml --ask-vault-pass
```

The playbook deliberately fails before changing the server if any required
secret remains a placeholder. It creates a one-time 3072-bit RSA signing key
at `/opt/aprendiendo-mcp/secrets/oauth-signing-key.pem`; preserve this file or
existing access tokens will stop validating after a replacement. The key is
readable only by the container's fixed non-root UID (`65532`).
