CREATE TABLE routing_project_contexts(
 root_key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, stamp TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)), updated_at INTEGER NOT NULL
);
CREATE TABLE routing_decisions(
 id TEXT PRIMARY KEY, root_key TEXT NOT NULL REFERENCES routing_project_contexts(root_key),
 context_fingerprint TEXT NOT NULL, goal_digest TEXT NOT NULL, task_type TEXT NOT NULL,
 decision_json TEXT NOT NULL CHECK(json_valid(decision_json)), created_at INTEGER NOT NULL
);
CREATE TABLE routing_handoffs(
 decision_id TEXT PRIMARY KEY REFERENCES routing_decisions(id),
 workflow_id TEXT NOT NULL UNIQUE REFERENCES workflows(id), created_at INTEGER NOT NULL
);
CREATE TABLE routing_feedback(
 decision_id TEXT PRIMARY KEY REFERENCES routing_decisions(id),
 rating TEXT NOT NULL CHECK(rating IN('HELPFUL','INACCURATE','UNAVAILABLE','BETTER_ALTERNATIVE','TOO_COMPLEX')),
 failure_class TEXT NOT NULL CHECK(failure_class IN('NONE','TOOL','SETUP','NETWORK','SELECTION','CANCELLED','DENIED','UNKNOWN')),
 chosen_id TEXT, execution_id TEXT REFERENCES execution_runs(id), execution_status TEXT,
 updated_at INTEGER NOT NULL
);
CREATE TABLE routing_evaluations(
 id TEXT PRIMARY KEY, suite_version TEXT NOT NULL, result_json TEXT NOT NULL CHECK(json_valid(result_json)),
 created_at INTEGER NOT NULL
);
CREATE INDEX routing_recent_decisions ON routing_decisions(created_at DESC);
INSERT INTO schema_migrations(version,name) VALUES(11,'project_routing');
PRAGMA user_version=11;
