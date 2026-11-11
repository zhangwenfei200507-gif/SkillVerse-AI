"""Check Schema 13 and compare synthetic data across the guest installer lifecycle."""

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3


FIXTURE = "phase19-guest-fixture"
ROWS = {
    "entities": ("id", FIXTURE),
    "learning_records": ("entity_id", FIXTURE),
    "workflows": ("id", FIXTURE),
    "workflow_versions": ("workflow_id", FIXTURE),
    "workflow_nodes": ("workflow_id", FIXTURE),
    "execution_runs": ("workflow_id", FIXTURE),
    "execution_steps": ("execution_id", FIXTURE),
    "execution_events": ("workflow_id", FIXTURE),
    "ai_model_providers": ("id", FIXTURE),
    "ai_models": ("id", FIXTURE),
    "ai_model_calls": ("id", FIXTURE),
    "ai_live_spend": ("request_id", FIXTURE),
}


def verify(db):
    schema = db.execute("PRAGMA user_version").fetchone()[0]
    integrity = db.execute("PRAGMA integrity_check").fetchall()
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    migrations = db.execute(
        "SELECT version, name, applied_at FROM schema_migrations ORDER BY version"
    ).fetchall()
    if schema != 13 or integrity != [("ok",)] or foreign_keys:
        raise ValueError("Expected Schema 13, integrity_check=ok and zero foreign-key violations.")
    if [row[0] for row in migrations] != list(range(1, 14)):
        raise ValueError("Migration journal is incomplete.")
    return {"schema": schema, "integrity": "ok", "foreignKeyViolations": 0,
            "migrationJournalSha256": digest(migrations)}


def digest(rows):
    return hashlib.sha256(json.dumps(rows, separators=(",", ":")).encode()).hexdigest()


def snapshot(db):
    result = verify(db)
    fingerprints = {}
    for table, (column, value) in ROWS.items():
        rows = db.execute(f"SELECT * FROM {table} WHERE {column} = ?", (value,)).fetchall()
        if len(rows) != 1:
            raise ValueError(f"Expected a single preserved synthetic row in {table}.")
        fingerprints[table] = digest(rows)
    result["syntheticRows"] = fingerprints
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("check", "seed", "compare"))
    parser.add_argument("database", type=Path)
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    uri = args.database.resolve(strict=True).as_uri()
    with closing(sqlite3.connect(uri + ("?mode=rw" if args.mode == "seed" else "?mode=ro"), uri=True)) as db:
        db.execute("PRAGMA foreign_keys=ON")
        result = verify(db)
        if args.mode == "seed":
            with db:
                db.executescript("BEGIN;\n" + Path(__file__).with_suffix(".sql").read_text(encoding="utf-8"))
                result = snapshot(db)
        if args.mode == "compare":
            result = snapshot(db)
            expected = json.loads(args.baseline.read_text(encoding="utf-8-sig"))
            if result != expected:
                raise ValueError("Synthetic rows or migration journal changed during lifecycle.")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
