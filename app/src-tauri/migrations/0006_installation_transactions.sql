BEGIN IMMEDIATE;
CREATE TABLE installation_operations (
 id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, entity_id TEXT NOT NULL REFERENCES entities(id),
 action TEXT NOT NULL, state TEXT NOT NULL, plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
 plan_digest TEXT NOT NULL, approved_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 error_json TEXT, recovery_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(recovery_json))
);
CREATE TABLE installation_operation_events (
 id INTEGER PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES installation_operations(id),
 state TEXT NOT NULL, details_json TEXT NOT NULL CHECK(json_valid(details_json)), created_at INTEGER NOT NULL
);
CREATE TABLE managed_installations (
 id TEXT PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES entities(id), profile_id TEXT NOT NULL,
 kind TEXT NOT NULL, scope TEXT NOT NULL, target_key TEXT NOT NULL UNIQUE, record_json TEXT NOT NULL CHECK(json_valid(record_json)),
 state TEXT NOT NULL, current_operation_id TEXT NOT NULL REFERENCES installation_operations(id), updated_at INTEGER NOT NULL
);
CREATE TABLE installation_target_locks (
 target_key TEXT PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES installation_operations(id)
);
CREATE INDEX installation_operation_profile ON installation_operations(profile_id,created_at);
CREATE INDEX managed_installation_entity ON managed_installations(entity_id,profile_id);
INSERT INTO schema_migrations(version,name) VALUES(6,'installation_transactions');
PRAGMA user_version=6;
COMMIT;
