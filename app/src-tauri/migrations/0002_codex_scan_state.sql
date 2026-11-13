BEGIN;

CREATE TABLE codex_scan_state (
    id TEXT PRIMARY KEY CHECK (id = 'latest'),
    scanned_at_unix_ms INTEGER NOT NULL CHECK (scanned_at_unix_ms >= 0),
    report_json TEXT NOT NULL CHECK (json_valid(report_json)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT INTO schema_migrations(version, name) VALUES (2, 'codex_scan_state');
PRAGMA user_version = 2;
COMMIT;
