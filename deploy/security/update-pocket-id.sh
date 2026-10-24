#!/usr/bin/env bash
# Run as root on the shared production host. Keep backups root-readable only.
set -Eeuo pipefail
[[ $(id -u) == 0 ]] || { echo 'Run as root' >&2; exit 1; }
cd /opt/fitsync/app
image='ghcr.io/pocket-id/pocket-id:v2.19.1@sha256:f085157f0b6ea4b2e8aa5ecfd99eada0d83a4960f3a3dcfd46014097d0ef7ab5'
docker pull "$image"
backup="/var/backups/pocket-id-security-$(date -u +%Y%m%dT%H%M%SZ)"
install -d -m 0700 "$backup"
cp -p compose.production.yaml "$backup/compose.production.yaml"
# Stop writers so the tar includes a consistent database, WAL, and uploads.
docker compose -f compose.production.yaml stop pocketid
trap 'echo "Upgrade failed; consistent recovery backup: $backup. Restore both data and compose before downgrading." >&2' ERR
tar -C /opt/fitsync -cpf "$backup/pocketid-data.tar" data/pocketid secrets/pocketid_encryption_key
chmod 0600 "$backup/pocketid-data.tar"
python3 - "$image" <<'PY'
from pathlib import Path
import sys
p = Path('compose.production.yaml')
s = p.read_text()
start = s.index('  pocketid:\n')
end = s.index('\n  fitnessd:', start)
section = s[start:end]
lines = section.splitlines()
for i, line in enumerate(lines):
    if line.startswith('    image:'):
        lines[i] = '    image: ' + sys.argv[1]
section = '\n'.join(lines) + '\n'
if '    user:' not in section:
    section = section.replace('    restart: unless-stopped\n', '''    restart: unless-stopped
    user: "1000:1000"
    read_only: true
    tmpfs:
      - /tmp:size=16m,mode=1777
    cap_drop:
      - ALL
    security_opt:
      - no-new-privileges:true
    pids_limit: 256
    mem_limit: 256m
''')
p.write_text(s[:start] + section + s[end:])
PY
docker compose -f compose.production.yaml config --quiet
docker compose -f compose.production.yaml up -d --no-deps --wait --wait-timeout 120 pocketid
echo "Pocket ID upgraded; consistent backup: $backup"
