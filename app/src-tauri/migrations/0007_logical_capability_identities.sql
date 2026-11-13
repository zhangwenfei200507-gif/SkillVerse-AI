-- Applied only inside an explicitly approved reconciliation transaction.
-- Startup intentionally leaves existing schema 6 databases unchanged.
CREATE TABLE capability_origins(origin_key TEXT PRIMARY KEY, kind TEXT NOT NULL, authority_json TEXT NOT NULL CHECK(json_valid(authority_json)),created_operation_id TEXT);
CREATE TABLE logical_capabilities(logical_id TEXT PRIMARY KEY,kind TEXT NOT NULL,machine_name TEXT NOT NULL,origin_key TEXT NOT NULL REFERENCES capability_origins(origin_key),component_key TEXT NOT NULL,confidence TEXT NOT NULL,created_at INTEGER NOT NULL,created_operation_id TEXT);
CREATE TABLE capability_physical_installations(physical_id TEXT PRIMARY KEY,entity_id TEXT NOT NULL REFERENCES entities(id),logical_id TEXT NOT NULL REFERENCES logical_capabilities(logical_id),scope_key TEXT NOT NULL,location TEXT NOT NULL,manifest_path TEXT,package_version TEXT,cache_version TEXT,manifest_hash TEXT NOT NULL,present INTEGER NOT NULL CHECK(present IN (0,1)),status TEXT NOT NULL,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL,evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)),last_operation_id TEXT);
CREATE INDEX capability_physical_logical ON capability_physical_installations(logical_id,present);
CREATE INDEX capability_physical_entity ON capability_physical_installations(entity_id);
CREATE TABLE capability_entity_links(entity_id TEXT PRIMARY KEY REFERENCES entities(id),logical_id TEXT NOT NULL REFERENCES logical_capabilities(logical_id),proof_json TEXT NOT NULL CHECK(json_valid(proof_json)),last_operation_id TEXT);
CREATE TABLE capability_scan_history(scan_id TEXT PRIMARY KEY,scanned_at INTEGER NOT NULL,context_json TEXT NOT NULL CHECK(json_valid(context_json)),report_json TEXT NOT NULL CHECK(json_valid(report_json)),imported INTEGER NOT NULL DEFAULT 0,operation_id TEXT);
CREATE TABLE capability_scan_observations(scan_id TEXT NOT NULL REFERENCES capability_scan_history(scan_id),physical_id TEXT NOT NULL REFERENCES capability_physical_installations(physical_id),entity_id TEXT NOT NULL REFERENCES entities(id),status TEXT NOT NULL,PRIMARY KEY(scan_id,physical_id));
CREATE TABLE capability_reconciliations(plan_digest TEXT PRIMARY KEY,approved_at INTEGER NOT NULL,backup_json TEXT NOT NULL CHECK(json_valid(backup_json)),preview_json TEXT NOT NULL CHECK(json_valid(preview_json)));
INSERT INTO schema_migrations(version,name) VALUES(7,'logical_capability_identities');
PRAGMA user_version=7;
