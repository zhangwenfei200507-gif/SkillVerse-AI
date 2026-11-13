BEGIN;

CREATE TABLE github_api_cache (
    cache_key TEXT PRIMARY KEY,
    etag TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    fetched_at_unix_ms INTEGER NOT NULL CHECK (fetched_at_unix_ms >= 0),
    response_status INTEGER NOT NULL DEFAULT 200
);

INSERT INTO schema_migrations(version, name) VALUES (3, 'github_discovery_cache');
PRAGMA user_version = 3;
COMMIT;
