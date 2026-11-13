CREATE TABLE ai_model_providers(
 id TEXT PRIMARY KEY, config_json TEXT NOT NULL CHECK(json_valid(config_json)), revision TEXT NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE ai_models(
 id TEXT PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES ai_model_providers(id),
 config_json TEXT NOT NULL CHECK(json_valid(config_json)), revision TEXT NOT NULL,
 health_json TEXT NOT NULL CHECK(json_valid(health_json)), cooldown_until INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL
);
CREATE TABLE ai_model_calls(
 id TEXT PRIMARY KEY, model_id TEXT NOT NULL REFERENCES ai_models(id), model_revision TEXT NOT NULL,
 request_digest TEXT NOT NULL, day TEXT NOT NULL, status TEXT NOT NULL, owner_id TEXT NOT NULL,
 reserved_requests INTEGER NOT NULL, reserved_tokens INTEGER NOT NULL, reserved_microusd INTEGER,
 attempts INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER, output_tokens INTEGER,
 error_code TEXT, response_json TEXT CHECK(response_json IS NULL OR json_valid(response_json)),
 created_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE INDEX ai_model_calls_budget ON ai_model_calls(model_id,day);
CREATE TABLE ai_model_cache(
 cache_key TEXT PRIMARY KEY, model_id TEXT NOT NULL REFERENCES ai_models(id), model_revision TEXT NOT NULL,
 response_json TEXT NOT NULL CHECK(json_valid(response_json)), expires_at INTEGER NOT NULL
);
INSERT INTO schema_migrations(version,name) VALUES(12,'model_registry');
PRAGMA user_version=12;
