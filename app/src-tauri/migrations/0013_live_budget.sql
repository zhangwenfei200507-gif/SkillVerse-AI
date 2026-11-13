CREATE TABLE ai_live_spend (
 request_id TEXT PRIMARY KEY REFERENCES ai_model_calls(id),
 provider_id TEXT NOT NULL,
 model_id TEXT NOT NULL,
 intent_digest TEXT NOT NULL,
 purpose TEXT NOT NULL CHECK(purpose IN ('acceptance','planning','classification','recommend','compose','learning','compare')),
 input_price INTEGER NOT NULL, cached_price INTEGER NOT NULL, output_price INTEGER NOT NULL,
 reserved_microusd INTEGER NOT NULL CHECK(reserved_microusd BETWEEN 1 AND 20000),
 settled_microusd INTEGER CHECK(settled_microusd >= 0),
 cached_tokens INTEGER, reasoning_tokens INTEGER,
 reported_model TEXT,
 input_tokens INTEGER, output_tokens INTEGER
);
CREATE INDEX ai_live_spend_acceptance ON ai_live_spend(purpose);
CREATE INDEX ai_live_spend_intent ON ai_live_spend(intent_digest);

CREATE TABLE ai_live_setup (id INTEGER PRIMARY KEY CHECK(id=1));
INSERT INTO schema_migrations(version,name) VALUES(13,'live_budget');
PRAGMA user_version=13;
