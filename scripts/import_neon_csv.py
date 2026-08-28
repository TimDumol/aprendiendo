#!/usr/bin/env python3
"""Build the SQLite database from CSV files exported from Neon with psql \\copy."""
import csv
import sqlite3
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
target = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "data/aprendiendo.sqlite3"
target.parent.mkdir(parents=True, exist_ok=True)
connection = sqlite3.connect(target)
connection.executescript((root / "sql/sqlite_schema.sql").read_text())
connection.execute("PRAGMA foreign_keys=OFF")
for table in ("sessions", "attempts", "weaknesses", "observations"):
    with (root / f"data/neon-{table}.csv").open(newline="") as handle:
        reader = csv.DictReader(handle)
        columns = reader.fieldnames or []
        placeholders = ",".join("?" for _ in columns)
        sql = f"INSERT OR REPLACE INTO {table} ({','.join(columns)}) VALUES ({placeholders})"
        rows = []
        for row in reader:
            values = [None if row[c] == "" else row[c] for c in columns]
            if table == "weaknesses":
                active = columns.index("active")
                values[active] = 1 if values[active] == "t" else 0
            rows.append(values)
        connection.executemany(sql, rows)
connection.execute("PRAGMA foreign_keys=ON")
violations = connection.execute("PRAGMA foreign_key_check").fetchall()
if violations:
    raise SystemExit(f"foreign-key violations: {violations}")
connection.commit()
for table in ("sessions", "attempts", "weaknesses", "observations"):
    print(f"{table}: {connection.execute(f'SELECT count(*) FROM {table}').fetchone()[0]}")
