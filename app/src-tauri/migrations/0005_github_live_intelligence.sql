BEGIN IMMEDIATE;
ALTER TABLE github_api_cache ADD COLUMN last_modified TEXT;
ALTER TABLE github_api_cache ADD COLUMN validated_at_unix_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE github_api_cache ADD COLUMN expires_at_unix_ms INTEGER NOT NULL DEFAULT 0;
CREATE TABLE github_rate_state (
 scope TEXT NOT NULL, resource TEXT NOT NULL, remaining INTEGER, limit_value INTEGER,
 reset_at INTEGER NOT NULL DEFAULT 0, blocked_until INTEGER NOT NULL DEFAULT 0,
 last_request_at INTEGER NOT NULL DEFAULT 0, failures INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(scope,resource)
);
CREATE TABLE github_repositories (
 id INTEGER PRIMARY KEY, full_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
 metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
 fetched_at INTEGER NOT NULL, validated_at INTEGER NOT NULL,
 availability TEXT NOT NULL DEFAULT 'AVAILABLE'
);
CREATE TABLE github_repository_aliases (
 normalized_name TEXT PRIMARY KEY COLLATE NOCASE,
 repository_id INTEGER NOT NULL REFERENCES github_repositories(id)
);
CREATE TABLE github_capability_sources (
 entity_id TEXT PRIMARY KEY REFERENCES entities(id),
 repository_id INTEGER NOT NULL REFERENCES github_repositories(id),
 source_path TEXT NOT NULL, kind TEXT NOT NULL,
 UNIQUE(repository_id,source_path,kind)
);
CREATE TABLE github_capability_versions (
 id INTEGER PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES entities(id),
 commit_sha TEXT NOT NULL, blob_sha TEXT NOT NULL, package_fingerprint TEXT,
 manifest_version TEXT, metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
 observed_at INTEGER NOT NULL, UNIQUE(entity_id,commit_sha,blob_sha)
);
CREATE TABLE github_capability_intelligence (
 entity_id TEXT PRIMARY KEY REFERENCES entities(id),
 payload_json TEXT NOT NULL CHECK(json_valid(payload_json)), checked_at INTEGER NOT NULL
);
CREATE TABLE github_classifications (
 entity_id TEXT PRIMARY KEY REFERENCES entities(id), primary_function TEXT,
 related_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(related_json)),
 state TEXT NOT NULL, confidence REAL NOT NULL CHECK(confidence>=0 AND confidence<=1),
 evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)),
 manual INTEGER NOT NULL DEFAULT 0 CHECK(manual IN(0,1)), checked_at INTEGER NOT NULL
);
CREATE TABLE github_duplicate_links (
 left_entity_id TEXT NOT NULL REFERENCES entities(id),
 right_entity_id TEXT NOT NULL REFERENCES entities(id),
 reason TEXT NOT NULL, checked_at INTEGER NOT NULL,
 PRIMARY KEY(left_entity_id,right_entity_id,reason), CHECK(left_entity_id!=right_entity_id)
);
CREATE TABLE github_local_links (
 local_entity_id TEXT NOT NULL REFERENCES entities(id),
 remote_entity_id TEXT NOT NULL REFERENCES entities(id), evidence_json TEXT NOT NULL,
 checked_at INTEGER NOT NULL, PRIMARY KEY(local_entity_id,remote_entity_id)
);
CREATE TABLE github_sync_runs (
 id TEXT PRIMARY KEY, operation_json TEXT NOT NULL CHECK(json_valid(operation_json)),
 status TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER,
 retry_at INTEGER, summary_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(summary_json))
);
CREATE TABLE github_discovery_events (
 id INTEGER PRIMARY KEY, run_id TEXT NOT NULL REFERENCES github_sync_runs(id),
 entity_id TEXT, event_type TEXT NOT NULL, details_json TEXT NOT NULL CHECK(json_valid(details_json)),
 created_at INTEGER NOT NULL
);
CREATE INDEX github_versions_entity ON github_capability_versions(entity_id,observed_at);
CREATE INDEX github_events_run ON github_discovery_events(run_id,id);
INSERT INTO schema_migrations(version,name) VALUES(5,'github_live_intelligence');
PRAGMA user_version=5;
COMMIT;
