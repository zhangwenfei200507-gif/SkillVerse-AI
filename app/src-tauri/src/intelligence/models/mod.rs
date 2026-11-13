//! Registration and budgeted advice use the existing Phase 7 Provider and
//! credential/transport boundary. This service cannot execute workflow nodes.
pub(super) mod live;
mod requests;
#[cfg(test)]
mod tests;
use super::*;
use std::collections::BTreeSet;

pub(super) type ModelResult<T> = std::result::Result<T, String>;
pub(super) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MIGRATION: &str = include_str!("../../../migrations/0012_model_registry.sql");
const LIVE_MIGRATION: &str = include_str!("../../../migrations/0013_live_budget.sql");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Feature {
    Text,
    Reasoning,
    ToolSelection,
    StructuredOutput,
    Embedding,
    LongContext,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub base_url: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelConfig {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub enabled: bool,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub timeout_seconds: u32,
    pub max_attempts: u32,
    pub daily_requests: u32,
    pub daily_tokens: u32,
    pub daily_budget_microusd: Option<u64>,
    pub task_tokens: u32,
    pub task_budget_microusd: Option<u64>,
    pub input_price_per_million_microusd: Option<u64>,
    pub output_price_per_million_microusd: Option<u64>,
    pub declared_features: BTreeSet<Feature>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Health {
    pub observed: BTreeSet<Feature>,
    pub source: String,
    pub checked_at: i64,
    pub last_error: Option<String>,
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
fn safe_label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 150 && s == clean(s) && !s.chars().any(char::is_control)
}
impl ProviderConfig {
    fn validate(&self) -> ModelResult<()> {
        if self.base_url.len() > 2048
            || !identifier(&self.id)
            || !safe_label(&self.name)
            || !matches!(
                self.transport.as_str(),
                "mock" | "openai-compatible" | "local"
            )
        {
            return Err("MODEL_PROVIDER_CONFIG_INVALID".into());
        }
        if self.transport == "mock" {
            if !self.base_url.is_empty() {
                return Err("MOCK_CANNOT_USE_NETWORK".into());
            }
        } else {
            self.settings("validation", 100, 1).validate()?;
        }
        Ok(())
    }
    pub(super) fn settings(&self, model: &str, output: u32, timeout: u32) -> Settings {
        Settings {
            provider: self.transport.clone(),
            base_url: self.base_url.clone(),
            model: model.into(),
            enabled: self.enabled,
            daily_requests: 10,
            daily_tokens: 100000,
            max_output_tokens: output,
            timeout_seconds: timeout,
        }
    }
}
impl ModelConfig {
    fn validate(&self) -> ModelResult<()> {
        if !identifier(&self.id)
            || !identifier(&self.provider_id)
            || !safe_label(&self.model_id)
            || !(1024..=2_000_000).contains(&self.context_window)
            || !(64..=4000).contains(&self.max_output_tokens)
            || self.max_output_tokens >= self.context_window
            || !(1..=60).contains(&self.timeout_seconds)
            || !(1..=2).contains(&self.max_attempts)
            || !(1..=1000).contains(&self.daily_requests)
            || !(1024..=2_000_000).contains(&self.daily_tokens)
            || !(64..=2_000_000).contains(&self.task_tokens)
            || [self.daily_budget_microusd, self.task_budget_microusd]
                .into_iter()
                .flatten()
                .any(|n| n > 1_000_000_000)
            || [
                self.input_price_per_million_microusd,
                self.output_price_per_million_microusd,
            ]
            .into_iter()
            .flatten()
            .any(|n| n > 10_000_000_000)
        {
            return Err("MODEL_CONFIG_INVALID".into());
        }
        Ok(())
    }
    pub(super) fn maximum_cost(&self, input: u64, output: u64) -> Option<u64> {
        let (a, b) = (
            self.input_price_per_million_microusd?,
            self.output_price_per_million_microusd?,
        );
        input
            .checked_mul(a)?
            .checked_add(output.checked_mul(b)?)?
            .checked_add(999999)
            .map(|n| n / 1_000_000)
    }
}

impl Database {
    pub(super) async fn model_entry(
        &self,
        id: &str,
    ) -> ModelResult<(ProviderConfig, ModelConfig, Health, String, i64)> {
        let row=sqlx::query("SELECT m.config_json,m.health_json,m.revision,m.cooldown_until,p.config_json AS provider FROM ai_models m JOIN ai_model_providers p ON p.id=m.provider_id WHERE m.id=?")
            .bind(id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?.ok_or("MODEL_NOT_FOUND")?;
        Ok((
            serde_json::from_str(row.get("provider")).map_err(|_| "MODEL_CONFIG_CORRUPT")?,
            serde_json::from_str(row.get("config_json")).map_err(|_| "MODEL_CONFIG_CORRUPT")?,
            serde_json::from_str(row.get("health_json")).map_err(|_| "MODEL_HEALTH_CORRUPT")?,
            row.get("revision"),
            row.get("cooldown_until"),
        ))
    }
    pub(crate) async fn model_route(&self, complex: bool) -> ModelResult<Value> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        if version < 12 {
            return Ok(
                json!({"modelId":null,"reason":"Enable model registry first","liveModel":"NOT_VERIFIED"}),
            );
        }
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM ai_models ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        let available = if version >= 13 {
            let mut connection = self.pool.acquire().await.map_err(|_| "DATABASE")?;
            let (month, today, _, _, _) = live::totals(&mut connection, &requests::day()).await?;
            live::MONTH
                .saturating_sub(month)
                .min(live::DAY.saturating_sub(today))
                .max(0) as u64
        } else {
            0
        };
        let mut eligible = Vec::new();
        for id in ids {
            let (p, m, h, _, cool) = self.model_entry(&id).await?;
            let needed = if complex {
                vec![
                    Feature::Text,
                    Feature::StructuredOutput,
                    Feature::ToolSelection,
                    Feature::Reasoning,
                ]
            } else {
                vec![
                    Feature::Text,
                    Feature::StructuredOutput,
                    Feature::ToolSelection,
                ]
            };
            if p.enabled
                && m.enabled
                && cool <= now()
                && needed
                    .iter()
                    .all(|f| h.observed.contains(f) && m.declared_features.contains(f))
            {
                let price = if p.transport == "openai-compatible" {
                    if h.source != "LIVE_PROBE_VERIFIED"
                        || (!complex && matches!(m.id.as_str(), "phase13-terra" | "phase13-pro"))
                    {
                        continue;
                    }
                    let Ok(tariff) = live::tariff(&p, &m, &requests::day()) else {
                        continue;
                    };
                    if !live::credential(&p.base_url)
                        .read()
                        .map_err(|_| "CREDENTIAL_STORE")?
                        .is_some()
                    {
                        continue;
                    }
                    let Some(estimated) = tariff.cost(3000, 0, u64::from(m.max_output_tokens))
                    else {
                        continue;
                    };
                    if estimated > available.min(live::TASK) {
                        continue;
                    }
                    estimated
                } else {
                    m.maximum_cost(1000, 1000).unwrap_or(u64::MAX)
                };
                // Verified metered slots take priority over explicitly labelled local fixtures.
                let fixture = p.transport != "openai-compatible";
                eligible.push((fixture, price, id, h.source));
            }
        }
        eligible.sort();
        Ok(if let Some((_, price, id, source)) = eligible.first() {
            json!({"modelId":id,"reason":if complex{"Required reasoning evidence, sufficient shared budget, then lowest estimated cost"}else{"Verified low-cost slots first, ranked by estimated cost"},"estimatedMicrousdPer1000InputAndOutput":if *price==u64::MAX{None}else{Some(*price)},"evidenceSource":source,"liveModel":"NOT_VERIFIED"})
        } else {
            json!({"modelId":null,"reason":"No enabled model has the required observed features; deterministic routing remains available","liveModel":"NOT_VERIFIED"})
        })
    }
    async fn model_status(&self) -> ModelResult<Value> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        if version < 12 {
            return Ok(
                json!({"ready":false,"schema":version,"providers":[],"models":[],"liveModel":"NOT_VERIFIED"}),
            );
        }
        let budget = if version >= 13 {
            let mut connection = self.pool.acquire().await.map_err(|_| "DATABASE")?;
            let (month, today, acceptance, settled, uncertain) =
                live::totals(&mut connection, &requests::day()).await?;
            json!({"monthlyLimit":live::MONTH,"dailyLimit":live::DAY,"taskLimit":live::TASK,
                "acceptanceLimit":live::ACCEPTANCE,"monthCommitted":month,"dayCommitted":today,
                "acceptanceCommitted":acceptance,"settledMonth":settled,"unsettled":uncertain,
                "monthRemaining":(live::MONTH-month).max(0),"dayRemaining":(live::DAY-today).max(0)})
        } else {
            Value::Null
        };
        self.recover_model_calls().await?;
        let providers: Vec<String> =
            sqlx::query_scalar("SELECT config_json FROM ai_model_providers ORDER BY id")
                .fetch_all(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
        let mut provider_status = Vec::new();
        for raw in providers {
            let provider: ProviderConfig =
                serde_json::from_str(&raw).map_err(|_| "MODEL_CONFIG_CORRUPT")?;
            let mut row = json!(provider);
            if provider.id == "phase13-openai" || provider.id == "phase13-deepseek" {
                row["credentialPresent"] = json!(
                    live::credential(&provider.base_url)
                        .read()
                        .map_err(|_| "CREDENTIAL_STORE")?
                        .is_some()
                );
            }
            provider_status.push(row);
        }
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM ai_models ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        let mut models = Vec::new();
        for id in ids {
            let (p, m, h, revision, cool) = self.model_entry(&id).await?;
            models.push(json!({"config":m,"health":h,"revision":revision,"transport":p.transport,"cooldownUntil":cool}));
        }
        let rows=sqlx::query("SELECT id,model_id,status,attempts,input_tokens,output_tokens,reserved_microusd,error_code,created_at FROM ai_model_calls ORDER BY created_at DESC,id LIMIT 20").fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
        let history:Vec<_>=rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"modelId":r.get::<String,_>("model_id"),"status":r.get::<String,_>("status"),"attempts":r.get::<i64,_>("attempts"),"inputTokens":r.get::<Option<i64>,_>("input_tokens"),"outputTokens":r.get::<Option<i64>,_>("output_tokens"),"costUpperBoundMicrousd":r.get::<Option<i64>,_>("reserved_microusd"),"error":r.get::<Option<String>,_>("error_code"),"createdAt":r.get::<i64,_>("created_at")})).collect();
        let mut ledger = Vec::new();
        if version >= 13 {
            let rows = sqlx::query("SELECT s.* FROM ai_live_spend s JOIN ai_model_calls c ON c.id=s.request_id ORDER BY c.created_at DESC,c.id LIMIT 20")
                .fetch_all(&self.pool).await.map_err(|_| "DATABASE")?;
            ledger.extend(rows.iter().map(|r| json!({
                "requestId":r.get::<String,_>("request_id"), "providerId":r.get::<String,_>("provider_id"),
                "modelId":r.get::<String,_>("model_id"), "purpose":r.get::<String,_>("purpose"),
                "reportedModel":r.get::<Option<String>,_>("reported_model"),
                "reservedMicrousd":r.get::<i64,_>("reserved_microusd"), "settledMicrousd":r.get::<Option<i64>,_>("settled_microusd"),
                "cachedTokens":r.get::<Option<i64>,_>("cached_tokens"), "reasoningTokens":r.get::<Option<i64>,_>("reasoning_tokens")
            })));
        }
        let live_verified = models
            .iter()
            .any(|model| model["health"]["source"] == "LIVE_PROBE_VERIFIED");
        Ok(
            json!({"ready":true,"schema":version,"budget":budget,"providers":provider_status,"models":models,"history":history,"liveLedger":ledger,"suggestedSimple":self.model_route(false).await?,"suggestedComplex":self.model_route(true).await?,"liveModel":if live_verified{"AT_LEAST_ONE_MODEL_PROBED"}else{"NOT_VERIFIED"},"budgetDay":"UTC","prices":"ESTIMATED_FROM_REPORTED_USAGE"}),
        )
    }
    pub(crate) async fn model_intelligence(&self, action: &str, args: Value) -> ModelResult<Value> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        if action == "model-status" {
            return self.model_status().await;
        }
        if action == "model-initialize" {
            if args["confirmed"] != true {
                return Err("MODEL_EXPLICIT_CONFIRMATION_REQUIRED".into());
            }
            let _lock = SERIAL.lock().await;
            if version >= 12 {
                return self.model_status().await;
            }
            if version != 11 {
                return Err("MODEL_REQUIRES_ROUTING_SCHEMA_11".into());
            }
            return self.protected_ai_migration(11, 12, MIGRATION).await;
        }
        if action == "model-live-initialize" {
            if args["confirmed"] != true {
                return Err("MODEL_EXPLICIT_CONFIRMATION_REQUIRED".into());
            }
            let _lock = SERIAL.lock().await;
            let migration = if version == 12 {
                Some(self.protected_ai_migration(12, 13, LIVE_MIGRATION).await?)
            } else if version == 13 {
                None
            } else {
                return Err("LIVE_REQUIRES_MODEL_SCHEMA_12".into());
            };
            let mut tx = self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|_| "DATABASE")?;
            let configured: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ai_live_setup WHERE id=1)")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            if !configured {
                for statement in include_str!("live_slots.sql")
                    .trim_end_matches(|c: char| c.is_whitespace() || c == ';')
                    .split(';')
                {
                    sqlx::query(statement)
                        .execute(&mut *tx)
                        .await
                        .map_err(|_| "LIVE_SLOT_ID_COLLISION")?;
                }
                sqlx::query("INSERT INTO ai_live_setup(id) VALUES(1)")
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            return Ok(migration.unwrap_or_else(|| json!({"schema":13,"ready":true})));
        }
        if version < 12 {
            return Err("MODEL_REGISTRY_NOT_INITIALIZED".into());
        }
        if action == "model-cancel" {
            return self.cancel_model_request(&args).await;
        }
        if action == "model-route" {
            return self.model_route(args["complex"] == true).await;
        }
        if action == "model-health" {
            return self.probe_model(&args).await;
        }
        if args["confirmed"] != true {
            return Err("MODEL_EXPLICIT_CONFIRMATION_REQUIRED".into());
        }
        let _lock = SERIAL.lock().await;
        if action == "model-save-provider" {
            let p: ProviderConfig = serde_json::from_value(args["provider"].clone())
                .map_err(|_| "MODEL_PROVIDER_CONFIG_INVALID")?;
            p.validate()?;
            if p.id.starts_with("phase13-") {
                return Err("LIVE_SLOT_IS_MANAGED".into());
            }
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM ai_model_providers WHERE id<>?")
                    .bind(&p.id)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|_| "DATABASE")?;
            if count >= 16 {
                return Err("MODEL_PROVIDER_LIMIT".into());
            }
            let raw = json!(p).to_string();
            let revision = digest(raw.as_bytes());
            let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
            sqlx::query("INSERT INTO ai_model_providers VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET config_json=excluded.config_json,revision=excluded.revision,updated_at=excluded.updated_at")
                .bind(&p.id).bind(raw).bind(&revision).bind(now()).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            // Bind the model revision to both configs without accumulating
            // revision strings on repeated provider saves.
            let affected: Vec<(String, String)> =
                sqlx::query_as("SELECT id,config_json FROM ai_models WHERE provider_id=?")
                    .bind(&p.id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            for (id, model_raw) in affected {
                let next = digest(format!("{}:{model_raw}", json!(p)).as_bytes());
                sqlx::query("UPDATE ai_models SET health_json=?,revision=?,updated_at=? WHERE id=? AND revision<>?")
                    .bind(json!(Health::default()).to_string()).bind(&next).bind(now()).bind(id).bind(&next).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            return Ok(json!({"saved":true,"healthInvalidated":true}));
        }
        if action == "model-save-model" {
            let m: ModelConfig = serde_json::from_value(args["model"].clone())
                .map_err(|_| "MODEL_CONFIG_INVALID")?;
            m.validate()?;
            if m.id.starts_with("phase13-") || m.provider_id.starts_with("phase13-") {
                return Err("LIVE_SLOT_IS_MANAGED".into());
            }
            let p_raw: String =
                sqlx::query_scalar("SELECT config_json FROM ai_model_providers WHERE id=?")
                    .bind(&m.provider_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|_| "DATABASE")?
                    .ok_or("MODEL_PROVIDER_NOT_FOUND")?;
            let p: ProviderConfig =
                serde_json::from_str(&p_raw).map_err(|_| "MODEL_CONFIG_CORRUPT")?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ai_models WHERE id<>?")
                .bind(&m.id)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            if count >= 64 {
                return Err("MODEL_LIMIT".into());
            }
            let raw = json!(m).to_string();
            let revision = digest(format!("{p_raw}:{raw}").as_bytes());
            sqlx::query("INSERT INTO ai_models VALUES(?,?,?,?,?,0,?) ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id,config_json=excluded.config_json,revision=excluded.revision,health_json=excluded.health_json,updated_at=excluded.updated_at")
                .bind(&m.id).bind(&p.id).bind(raw).bind(revision).bind(json!(Health::default()).to_string()).bind(now()).execute(&self.pool).await.map_err(|_|"DATABASE")?;
            return Ok(json!({"saved":true,"observedFeatures":[],"health":"NOT_VERIFIED"}));
        }
        if action == "model-credential" {
            let id = args["providerId"]
                .as_str()
                .ok_or("MODEL_PROVIDER_NOT_FOUND")?;
            let raw: String =
                sqlx::query_scalar("SELECT config_json FROM ai_model_providers WHERE id=?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|_| "DATABASE")?
                    .ok_or("MODEL_PROVIDER_NOT_FOUND")?;
            let p: ProviderConfig =
                serde_json::from_str(&raw).map_err(|_| "MODEL_CONFIG_CORRUPT")?;
            if p.transport != "openai-compatible" {
                return Err("MODEL_CREDENTIAL_NOT_APPLICABLE".into());
            }
            if version < 13
                || !matches!(
                    (id, p.base_url.as_str()),
                    ("phase13-openai", "https://api.openai.com/v1")
                        | ("phase13-deepseek", "https://api.deepseek.com")
                )
            {
                return Err("LIVE_OFFICIAL_SLOT_REQUIRED".into());
            }
            let running: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ai_live_spend s JOIN ai_model_calls c ON c.id=s.request_id WHERE s.provider_id=? AND c.status='RUNNING')")
                .bind(id).fetch_one(&self.pool).await.map_err(|_|"DATABASE")?;
            if running {
                return Err("LIVE_PROVIDER_BUSY".into());
            }
            let store = live::credential(&p.base_url);
            let mut tx = self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|_| "DATABASE")?;
            sqlx::query(
                "UPDATE ai_models SET health_json=?, revision=? || ':' || id WHERE provider_id=?",
            )
            .bind(json!(Health::default()).to_string())
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
            tx.commit().await.map_err(|_| "DATABASE")?;
            if args["remove"] == true {
                store.remove().map_err(|_| "CREDENTIAL_STORE")?;
            } else {
                let value = args["value"]
                    .as_str()
                    .filter(|s| {
                        s.len() >= 8
                            && s.len() <= 2048
                            && s.is_ascii()
                            && !s.chars().any(char::is_whitespace)
                    })
                    .ok_or("INVALID_CREDENTIAL")?;
                store.save(value).map_err(|_| "CREDENTIAL_STORE")?;
            }
            return Ok(json!({"saved":true,"valueReturned":false}));
        }
        Err("MODEL_UNKNOWN_ACTION".into())
    }
}

pub(super) struct MockProvider;
impl AIClassificationProvider for MockProvider {
    fn classify(&self, _: &str, _: &str) -> Option<Classification> {
        None
    }
}
impl Provider for MockProvider {
    fn complete<'a>(
        &'a self,
        _: &'a Subject,
    ) -> Pin<Box<dyn Future<Output = Result<(Profile, Option<(u32, u32)>)>> + Send + 'a>> {
        Box::pin(async { Err("MOCK_ROUTING_ONLY".into()) })
    }
    fn route_advice<'a>(
        &'a self,
        input: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let preferred: Vec<&str> = input["preferred"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(3)
                .collect();
            let picks: Vec<_> = input["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| preferred.contains(&c["id"].as_str().unwrap_or("")))
                .take(3)
                .map(|c| json!({"id":c["id"],"version":c["version"],"reason":"PURPOSE_MATCH"}))
                .collect();
            Ok(json!({"schema_version":1,"confidence":0.5,"selections":picks}))
        })
    }
}
