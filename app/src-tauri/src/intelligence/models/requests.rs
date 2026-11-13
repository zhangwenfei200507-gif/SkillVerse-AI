use super::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};
static OWNER: LazyLock<String> =
    LazyLock::new(|| format!("{}:{}", std::process::id(), uuid::Uuid::new_v4()));
static ACTIVE: LazyLock<tokio::sync::Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(Default::default);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequestScope {
    pub request_id: String,
    pub model_id: String,
    #[serde(default)]
    pub allow_network: bool,
    pub max_tokens: u32,
    pub max_cost_microusd: u64,
    #[serde(default = "one")]
    pub max_attempts: u32,
    #[serde(default)]
    pub bypass_cache: bool,
}
fn one() -> u32 {
    1
}
pub(super) fn day() -> String {
    chrono::DateTime::from_timestamp_millis(now())
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}
fn error_code(value: &str) -> &str {
    let head = value.split(':').next().unwrap_or("");
    if [
        "RATE_LIMITED",
        "TIMEOUT",
        "NETWORK_ERROR",
        "INVALID_JSON",
        "INVALID_PROFILE_JSON",
        "RESPONSE_SCHEMA_CHANGED",
        "SENSITIVE_PROVIDER_RESPONSE",
        "SENSITIVE_PROVIDER_INPUT",
        "RESPONSE_TOO_LARGE",
        "API_KEY_REQUIRED",
        "CANCELLED",
        "MODEL_CHANGED",
        "MODEL_USAGE_EXCEEDS_RESERVATION",
        "MODEL_RESPONSE_REJECTED",
        "LIVE_REPORTED_MODEL_MISMATCH",
        "LIVE_USAGE_UNAVAILABLE",
        "PROVIDER_INVALID_REQUEST",
        "PROVIDER_AUTHENTICATION_FAILED",
        "PROVIDER_INSUFFICIENT_BALANCE",
        "PROVIDER_ACCESS_DENIED",
        "PROVIDER_MODEL_UNAVAILABLE",
        "PROVIDER_HTTP_ERROR",
    ]
    .contains(&head)
        || live::transient(head)
    {
        head
    } else {
        "MODEL_REQUEST_FAILED"
    }
}

impl Database {
    pub(super) async fn recover_model_calls(&self) -> ModelResult<()> {
        let owners: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT owner_id FROM ai_model_calls WHERE status='RUNNING' AND owner_id<>?",
        )
        .bind(&*OWNER)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "DATABASE")?;
        for owner in owners {
            let alive = owner
                .split_once(':')
                .and_then(|(pid, _)| pid.parse::<u32>().ok())
                .is_some_and(crate::orchestration::pid_alive);
            if !alive {
                sqlx::query("UPDATE ai_model_calls SET status='INTERRUPTED',error_code='PROCESS_INTERRUPTED',finished_at=? WHERE status='RUNNING' AND owner_id=?")
                    .bind(now()).bind(owner).execute(&self.pool).await.map_err(|_| "DATABASE")?;
            }
        }
        Ok(())
    }
    pub(super) async fn cancel_model_request(&self, args: &Value) -> ModelResult<Value> {
        let id = args["requestId"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or("MODEL_REQUEST_ID_INVALID")?
            .to_string();
        if let Some(flag) = ACTIVE.lock().await.get(&id) {
            flag.store(true, Ordering::SeqCst);
        }
        let changed=sqlx::query("UPDATE ai_model_calls SET status='CANCELLED',error_code='CANCELLED',finished_at=? WHERE id=? AND status='RUNNING'")
            .bind(now()).bind(&id).execute(&self.pool).await.map_err(|_|"DATABASE")?.rows_affected();
        Ok(
            json!({"cancelled":changed==1,"requestId":id,"billing":"Reservations retained when usage is unknown"}),
        )
    }

    pub(crate) async fn generate_model_advice<F>(
        &self,
        scope: &Value,
        input: &Value,
        require_health: bool,
        validate: F,
    ) -> ModelResult<Value>
    where
        F: Fn(&Value) -> ModelResult<()> + Send + Sync,
    {
        self.generate_budgeted_advice(
            scope,
            input,
            require_health,
            validate,
            "planning",
            live::ROUTING_SYSTEM,
        )
        .await
    }

    pub(in crate::intelligence) async fn generate_budgeted_advice<F>(
        &self,
        scope: &Value,
        input: &Value,
        require_health: bool,
        validate: F,
        purpose: &str,
        system: &str,
    ) -> ModelResult<Value>
    where
        F: Fn(&Value) -> ModelResult<()> + Send + Sync,
    {
        let scope: RequestScope =
            serde_json::from_value(scope.clone()).map_err(|_| "MODEL_SCOPE_INVALID")?;
        let id = uuid::Uuid::parse_str(&scope.request_id)
            .map_err(|_| "MODEL_REQUEST_ID_INVALID")?
            .to_string();
        if !identifier(&scope.model_id)
            || input.to_string().len() > 32768
            || !(1..=2).contains(&scope.max_attempts)
        {
            return Err("MODEL_SCOPE_INVALID".into());
        }
        let (provider, model, health, revision, cooldown) =
            self.model_entry(&scope.model_id).await?;
        if !provider.enabled || !model.enabled {
            return Err("MODEL_DISABLED".into());
        }
        let paid = provider.transport == "openai-compatible";
        if !paid && cooldown > now() {
            return Err("MODEL_RATE_LIMIT_COOLDOWN".into());
        }
        let mock = provider.transport == "mock";
        if !mock && !scope.allow_network {
            return Err("MODEL_NETWORK_AUTHORIZATION_REQUIRED".into());
        }
        if require_health
            && ((paid && health.source != "LIVE_PROBE_VERIFIED")
                || ![
                    Feature::Text,
                    Feature::StructuredOutput,
                    Feature::ToolSelection,
                ]
                .iter()
                .all(|f| health.observed.contains(f)))
        {
            return Err("MODEL_HEALTH_NOT_VERIFIED".into());
        }
        let request_day = day();
        let tariff = if paid {
            let version: i64 = sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            if version < 13 {
                return Err("LIVE_SHARED_BUDGET_REQUIRED".into());
            }
            let tariff = live::tariff(&provider, &model, &request_day)?;
            if !live::credential(&provider.base_url)
                .read()
                .map_err(|_| "CREDENTIAL_STORE")?
                .is_some()
            {
                return Err("API_KEY_REQUIRED".into());
            }
            Some(tariff)
        } else {
            None
        };
        let attempts = if paid {
            1
        } else {
            scope.max_attempts.min(model.max_attempts)
        };
        let input_bound = input.to_string().len() as u64
            + if paid {
                system.len() as u64 + 1024
            } else {
                2048
            };
        let token_bound = (input_bound + u64::from(model.max_output_tokens)) * u64::from(attempts);
        let cost = if mock {
            Some(0)
        } else if let Some(tariff) = &tariff {
            tariff.cost(input_bound, 0, u64::from(model.max_output_tokens))
        } else {
            model
                .maximum_cost(input_bound, u64::from(model.max_output_tokens))
                .and_then(|n| n.checked_mul(u64::from(attempts)))
        };
        if input_bound + u64::from(model.max_output_tokens) > u64::from(model.context_window)
            || token_bound > u64::from(scope.max_tokens.min(model.task_tokens))
        {
            return Err("MODEL_TASK_TOKEN_BUDGET".into());
        }
        if !mock && cost.is_none() {
            return Err("MODEL_PRICING_REQUIRED_FOR_NETWORK".into());
        }
        if cost.is_some_and(|n| {
            n > scope.max_cost_microusd || model.task_budget_microusd.is_some_and(|cap| n > cap)
        }) {
            return Err("MODEL_TASK_COST_BUDGET".into());
        }
        let key = digest(
            if !paid && purpose == "planning" {
                json!(["model-advice/v1", input, revision])
            } else {
                json!(["model-advice/v2", purpose, system, input, revision])
            }
            .to_string()
            .as_bytes(),
        );
        let request_digest = digest(
            json!([
                key,
                scope.allow_network,
                scope.max_tokens,
                scope.max_cost_microusd,
                attempts
            ])
            .to_string()
            .as_bytes(),
        );
        let flag = Arc::new(AtomicBool::new(false));
        let started = std::time::Instant::now();
        {
            let _lock = SERIAL.lock().await;
            self.recover_model_calls().await?;
            let mut tx = self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(|_| "DATABASE")?;
            let (current, cooldown): (String, i64) =
                sqlx::query_as("SELECT revision,cooldown_until FROM ai_models WHERE id=?")
                    .bind(&model.id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            if current != revision {
                return Err("MODEL_CHANGED".into());
            }
            if !paid && cooldown > now() {
                return Err("MODEL_RATE_LIMIT_COOLDOWN".into());
            }
            if let Some(row) = sqlx::query(
                "SELECT request_digest,status,response_json FROM ai_model_calls WHERE id=?",
            )
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?
            {
                if row.get::<&str, _>("request_digest") != request_digest {
                    return Err("MODEL_REQUEST_ID_CONFLICT".into());
                }
                if row.get::<&str, _>("status") == "SUCCEEDED"
                    && let Some(raw) = row.get::<Option<&str>, _>("response_json")
                {
                    let value: Value =
                        serde_json::from_str(raw).map_err(|_| "MODEL_CACHE_CORRUPT")?;
                    validate(&value)?;
                    return Ok(
                        json!({"value":value,"requestId":id,"modelId":model.id,"duplicate":true,"cacheHit":true,"cacheDigest":key,"source":if mock{"MOCK"}else{"LIVE_ADAPTER"},"provider":provider.id,"actualModelId":model.model_id,"estimatedMicrousd":0,"settledMicrousd":0,"externalCalls":0}),
                    );
                }
                return Err("MODEL_REQUEST_ALREADY_RECORDED".into());
            }
            if !scope.bypass_cache {
                let cached: Option<String> = sqlx::query_scalar(
                    "SELECT response_json FROM ai_model_cache WHERE cache_key=? AND expires_at>?",
                )
                .bind(&key)
                .bind(now())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
                if let Some(raw) = cached {
                    let value: Value =
                        serde_json::from_str(&raw).map_err(|_| "MODEL_CACHE_CORRUPT")?;
                    validate(&value)?;
                    sqlx::query("INSERT INTO ai_model_calls(id,model_id,model_revision,request_digest,day,status,owner_id,reserved_requests,reserved_tokens,reserved_microusd,response_json,created_at,finished_at) VALUES(?,?,?,?,?,'SUCCEEDED',?,0,0,0,?,?,?)")
                        .bind(&id).bind(&model.id).bind(&revision).bind(&request_digest).bind(&request_day).bind(&*OWNER).bind(raw).bind(now()).bind(now()).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
                    tx.commit().await.map_err(|_| "DATABASE")?;
                    return Ok(
                        json!({"value":value,"requestId":id,"modelId":model.id,"cacheHit":true,"cacheDigest":key,"source":if mock{"MOCK"}else{"LIVE_ADAPTER"},"provider":provider.id,"actualModelId":model.model_id,"estimatedMicrousd":0,"settledMicrousd":0,"externalCalls":0}),
                    );
                }
            }
            let sums:(i64,i64,i64)=sqlx::query_as("SELECT COALESCE(sum(reserved_requests),0),COALESCE(sum(reserved_tokens),0),COALESCE(sum(reserved_microusd),0) FROM ai_model_calls WHERE model_id=? AND day=?")
                .bind(&model.id).bind(&request_day).fetch_one(&mut *tx).await.map_err(|_|"DATABASE")?;
            if sums.0 + attempts as i64 > model.daily_requests as i64
                || sums.1 + token_bound as i64 > model.daily_tokens as i64
                || model
                    .daily_budget_microusd
                    .is_some_and(|cap| sums.2 as u64 + cost.unwrap_or(0) > cap)
            {
                return Err("MODEL_DAILY_BUDGET".into());
            }
            sqlx::query("INSERT INTO ai_model_calls(id,model_id,model_revision,request_digest,day,status,owner_id,reserved_requests,reserved_tokens,reserved_microusd,created_at) VALUES(?,?,?,?,?,'RUNNING',?,?,?,?,?)")
                .bind(&id).bind(&model.id).bind(&revision).bind(&request_digest).bind(&request_day).bind(&*OWNER).bind(attempts as i64).bind(token_bound as i64).bind(cost.map(|n|n as i64)).bind(now()).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            if let Some(tariff) = &tariff {
                let intent = digest(json!([purpose, input]).to_string().as_bytes());
                live::reserve(
                    &mut tx,
                    &id,
                    &provider,
                    &model,
                    purpose,
                    cost.ok_or("LIVE_PRICE_UNKNOWN")?,
                    tariff,
                    &request_day,
                    &intent,
                )
                .await?;
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            ACTIVE.lock().await.insert(id.clone(), flag.clone());
        }
        let mut calls = 0;
        let mut in_tokens = 0u64;
        let mut out_tokens = 0u64;
        let mut all_usage = true;
        let mut metadata = Value::Null;
        let mut retry_after = None;
        let mut outcome: ModelResult<Value> = Err("MODEL_REQUEST_FAILED".into());
        for attempt in 0..attempts {
            let status: String = sqlx::query_scalar("SELECT status FROM ai_model_calls WHERE id=?")
                .bind(&id)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            if flag.load(Ordering::SeqCst) || status != "RUNNING" {
                outcome = Err("CANCELLED".into());
                break;
            }
            calls += 1;
            sqlx::query("UPDATE ai_model_calls SET attempts=? WHERE id=? AND status='RUNNING'")
                .bind(calls)
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            let client = Compatible {
                settings: provider.settings(
                    &model.model_id,
                    model.max_output_tokens,
                    model.timeout_seconds,
                ),
            };
            let request = async {
                tokio::time::timeout(Duration::from_secs(model.timeout_seconds as u64), async {
                    if mock {
                        MockProvider
                            .routing_complete(input)
                            .await
                            .map(|(value, usage)| (value, usage, Value::Null))
                    } else {
                        client.request_json_policy(system, input, true, paid).await
                    }
                })
                .await
                .map_err(|_| "TIMEOUT".to_string())?
            };
            let cancelled = async {
                loop {
                    if flag.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            let reply =
                tokio::select! {biased; reply=request=>reply,_=cancelled=>Err("CANCELLED".into())};
            match reply {
                Ok((value, usage, reported_metadata)) => {
                    metadata = reported_metadata;
                    if let Some((a, b)) = usage {
                        in_tokens += u64::from(a);
                        out_tokens += u64::from(b);
                    } else {
                        all_usage = false;
                    }
                    outcome = if paid
                        && !live::reported_model_matches(
                            &model.model_id,
                            &metadata["reportedModel"],
                        ) {
                        Err("LIVE_REPORTED_MODEL_MISMATCH".into())
                    } else if paid
                        && (usage.is_none()
                            || metadata["cachedTokens"]
                                .as_u64()
                                .is_some_and(|cached| cached > in_tokens))
                    {
                        Err("LIVE_USAGE_UNAVAILABLE".into())
                    } else if in_tokens > input_bound
                        || out_tokens > u64::from(model.max_output_tokens)
                    {
                        Err("MODEL_USAGE_EXCEEDS_RESERVATION".into())
                    } else {
                        validate(&value)
                            .map(|_| value)
                            .map_err(|_| "MODEL_RESPONSE_REJECTED".into())
                    };
                    break;
                }
                Err(error) => {
                    all_usage = false;
                    retry_after = error
                        .strip_prefix("RATE_LIMITED:")
                        .and_then(|v| v.parse::<i64>().ok());
                    if let Some(seconds) = retry_after.filter(|_| !paid) {
                        sqlx::query(
                            "UPDATE ai_models SET cooldown_until=max(cooldown_until,?) WHERE id=?",
                        )
                        .bind(now().saturating_add(seconds.max(1).saturating_mul(1000)))
                        .bind(&model.id)
                        .execute(&self.pool)
                        .await
                        .map_err(|_| "DATABASE")?;
                    }
                    let retry = !paid && live::transient(&error) && attempt + 1 < attempts;
                    outcome = Err(error_code(&error).to_string());
                    if !retry {
                        break;
                    }
                    tokio::select! {_=tokio::time::sleep(Duration::from_millis(150))=>{},_=async{loop{if flag.load(Ordering::SeqCst){break}tokio::time::sleep(Duration::from_millis(10)).await;}}=>{outcome=Err("CANCELLED".into());break}}
                }
            }
        }
        ACTIVE.lock().await.remove(&id);
        if flag.load(Ordering::SeqCst) {
            outcome = Err("CANCELLED".into());
        }
        let _lock = SERIAL.lock().await;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| "DATABASE")?;
        let current: String = sqlx::query_scalar("SELECT revision FROM ai_models WHERE id=?")
            .bind(&model.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
        if current != revision {
            outcome = Err("MODEL_CHANGED".into());
        }
        let status = if outcome.is_ok() {
            "SUCCEEDED"
        } else if outcome.as_ref().err().is_some_and(|v| v == "CANCELLED") {
            "CANCELLED"
        } else {
            "FAILED"
        };
        let reported = all_usage && calls > 0;
        let charged_tokens = if reported {
            in_tokens + out_tokens
        } else {
            token_bound
        };
        let actual = if reported
            && live::reported_model_matches(&model.model_id, &metadata["reportedModel"])
        {
            tariff.as_ref().and_then(|tariff| {
                tariff.cost(
                    in_tokens,
                    metadata["cachedTokens"].as_u64().unwrap_or(0),
                    out_tokens,
                )
            })
        } else {
            None
        };
        let charged_cost = if mock {
            Some(0)
        } else if paid {
            actual.or(cost)
        } else if reported {
            model.maximum_cost(in_tokens, out_tokens)
        } else {
            cost
        };
        let changed=sqlx::query("UPDATE ai_model_calls SET status=?,attempts=?,reserved_requests=?,reserved_tokens=?,reserved_microusd=?,input_tokens=?,output_tokens=?,error_code=?,response_json=?,finished_at=? WHERE id=? AND status='RUNNING'")
            .bind(status).bind(calls).bind(if reported||outcome.is_ok(){calls}else{attempts as i32}).bind(charged_tokens as i64).bind(charged_cost.map(|v|v as i64))
            .bind(if reported{Some(in_tokens as i64)}else{None}).bind(if reported{Some(out_tokens as i64)}else{None})
            .bind(outcome.as_ref().err().map(String::as_str)).bind(outcome.as_ref().ok().map(Value::to_string)).bind(now()).bind(&id).execute(&mut *tx).await.map_err(|_|"DATABASE")?.rows_affected();
        if paid {
            if changed == 1 || actual.is_some() {
                live::settle(&mut tx, &id, actual, in_tokens, out_tokens, &metadata).await?;
            }
            if changed == 1 || retry_after.is_some() {
                live::update_provider_cooldown(
                    &mut tx,
                    &provider.id,
                    outcome.as_ref().err().map(String::as_str),
                    retry_after,
                    actual.is_some(),
                    now(),
                )
                .await?;
            }
        }
        if changed != 1 {
            tx.commit().await.map_err(|_| "DATABASE")?;
            return Err("CANCELLED".into());
        }
        if let Ok(value) = &outcome {
            sqlx::query("INSERT INTO ai_model_cache VALUES(?,?,?,?,?) ON CONFLICT(cache_key) DO UPDATE SET response_json=excluded.response_json,expires_at=excluded.expires_at")
                .bind(&key).bind(&model.id).bind(&revision).bind(value.to_string()).bind(now()+900000).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
        }
        tx.commit().await.map_err(|_| "DATABASE")?;
        let value = outcome?;
        Ok(
            json!({"value":value,"requestId":id,"modelId":model.id,"source":if mock{"MOCK"}else{"LIVE_ADAPTER"},"cacheHit":false,"cacheDigest":key,"attempts":calls,"estimatedTokens":token_bound,"inputTokens":if reported{Some(in_tokens)}else{None},"outputTokens":if reported{Some(out_tokens)}else{None},"usageState":if mock{"MOCK_NOT_BILLED"}else if reported{"PROVIDER_REPORTED"}else{"UNKNOWN_RESERVATION_RETAINED"},"costUpperBoundMicrousd":charged_cost,"estimatedMicrousd":cost,"settledMicrousd":actual,"purpose":purpose,"provider":provider.id,"actualModelId":model.model_id,"providerUsage":metadata,"elapsedMs":started.elapsed().as_millis(),"externalCalls":if mock{0}else{calls}}),
        )
    }

    pub(super) async fn probe_model(&self, args: &Value) -> ModelResult<Value> {
        if args["confirmed"] != true {
            return Err("MODEL_EXPLICIT_CONFIRMATION_REQUIRED".into());
        }
        let mut scope = args["scope"].clone();
        scope["bypassCache"] = json!(true);
        let id = scope["modelId"].as_str().ok_or("MODEL_NOT_FOUND")?;
        let (provider, model, _, revision, _) = self.model_entry(id).await?;
        let input = json!({"task":"请选择浏览器测试能力，不要选择文档提取。","candidates":[{"id":"probe-browser","version":"1","purpose":"Browser testing"},{"id":"probe-documents","version":"1","purpose":"Document extraction"}],"preferred":["probe-browser"]});
        let result = self
            .generate_budgeted_advice(
                &scope,
                &input,
                false,
                |v| {
                    let valid = v.as_object().is_some_and(|o| o.len() == 3)
                        && v["schema_version"] == 1
                        && v["confidence"]
                            .as_f64()
                            .is_some_and(|n| (0.0..=1.0).contains(&n))
                        && v["selections"].as_array().is_some_and(|a| {
                            a.len() == 1
                                && a[0].as_object().is_some_and(|o| o.len() == 3)
                                && a[0]["id"] == "probe-browser"
                                && a[0]["version"] == "1"
                                && matches!(
                                    a[0]["reason"].as_str(),
                                    Some("PURPOSE_MATCH" | "STACK_MATCH" | "COMPLEMENTARY")
                                )
                        });
                    if valid {
                        Ok(())
                    } else {
                        Err("MODEL_PROBE_FAILED".into())
                    }
                },
                "acceptance",
                live::ROUTING_SYSTEM,
            )
            .await;
        let mut observed = BTreeSet::new();
        if result.is_ok() {
            observed.extend([
                Feature::Text,
                Feature::StructuredOutput,
                Feature::ToolSelection,
            ]);
            if provider.transport == "mock"
                || result
                    .as_ref()
                    .ok()
                    .and_then(|r| r["providerUsage"]["reasoningTokens"].as_u64())
                    .is_some_and(|n| n > 0)
            {
                observed.insert(Feature::Reasoning);
            }
        }
        let health = Health {
            observed,
            source: if provider.transport == "mock" {
                "MOCK_FIXTURE"
            } else if provider.transport == "openai-compatible" && result.is_ok() {
                if model.model_id == "deepseek-v4-pro"
                    && !result
                        .as_ref()
                        .ok()
                        .and_then(|receipt| receipt["providerUsage"]["reportedModel"].as_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("DeepSeek-V4-Pro-0813"))
                {
                    "LIVE_PROBE_ROUTE_UNCONFIRMED"
                } else {
                    "LIVE_PROBE_VERIFIED"
                }
            } else {
                "SINGLE_ADAPTER_PROBE_NOT_FULL_ACCEPTANCE"
            }
            .into(),
            checked_at: now(),
            last_error: result.as_ref().err().map(|e| error_code(e).into()),
        };
        sqlx::query("UPDATE ai_models SET health_json=? WHERE id=? AND revision=?")
            .bind(json!(health).to_string())
            .bind(id)
            .bind(revision)
            .execute(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        result.map(|receipt|json!({"health":health,"receipt":receipt,"liveModel":if health.source == "LIVE_PROBE_VERIFIED" {"MODEL_RESPONSE_VERIFIED"} else {"NOT_VERIFIED"},"untested":["embedding","long_context","general_semantic_accuracy"]}))
    }
}
