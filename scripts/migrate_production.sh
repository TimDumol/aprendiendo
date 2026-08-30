#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--confirm-production" ]]; then
  echo "Refusing to mutate production without --confirm-production." >&2
  echo "Usage: $0 --confirm-production /path/to/aprendiendo.sqlite3" >&2
  exit 2
fi
if [[ $# -ne 2 || -z "${2}" ]]; then
  echo "Usage: $0 --confirm-production /path/to/aprendiendo.sqlite3" >&2
  exit 2
fi

database_path=$2
if [[ ! -f "$database_path" ]]; then
  echo "Database does not exist: $database_path" >&2
  exit 2
fi

if [[ -n "${MIGRATE_BIN:-}" ]]; then
  [[ -x "$MIGRATE_BIN" ]] || {
    echo "Migration binary is not executable: $MIGRATE_BIN" >&2
    exit 2
  }
  exec "$MIGRATE_BIN" "$database_path"
fi

exec cargo run --release --bin migrate -- "$database_path"
