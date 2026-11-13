//! Source-grounded learning and artifact verification, independent of installation.
mod artifacts;
#[cfg(test)]
mod tests;
use crate::{
    db::Database,
    github::{
        analysis,
        types::{digest, now},
    },
    intelligence::Subject,
};
use serde_json::{Value, json};
use sqlx::Row;
type Result<T> = std::result::Result<T, String>;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const STAGES: [&str; 5] = ["Explore", "Configure", "Practice", "Verify", "Master"];
fn mastery(runs: &[Value]) -> Value {
    let artifact_sessions: std::collections::BTreeSet<_> = runs
        .iter()
        .filter(|r| r["status"] == "PASSED" && r["evidenceIntact"] == true)
        .filter_map(|r| r["sessionId"].as_str())
        .collect();
    // Only a separately trusted executor can provide these layers; the public
    // artifact/attestation interfaces never accept them from client data.
    let execution_sessions: std::collections::BTreeSet<_> = runs
        .iter()
        .filter(|r| {
            r["status"] == "PASSED"
                && r["evidenceIntact"] == true
                && r["trust"] == "LOCAL_VALIDATOR"
                && r["method"] == "TRUSTED_CAPABILITY_EXECUTION_V1"
                && [
                    "capability_runtime",
                    "tool_invocation",
                    "task_artifact",
                    "user_performance",
                ]
                .iter()
                .all(|layer| r["layers"][*layer] == "PASSED")
        })
        .filter_map(|r| r["sessionId"].as_str())
        .collect();
    let independent_review = runs.iter().any(|r| {
        r["status"] == "PASSED"
            && r["evidenceIntact"] == true
            && r["method"] == "INDEPENDENT_PERFORMANCE_V1"
            && r["layers"]["user_performance"] == "PASSED"
    });
    let verified = execution_sessions.len() >= 3 && independent_review;
    json!({"status":if verified{"VERIFIED"}else{"NOT_VERIFIED"},"artifactPasses":artifact_sessions.len(),"trustedExecutionSessions":execution_sessions.len(),"independentPerformanceReview":independent_review,"requiredDistinctSessions":3,"reason":if verified{"Multiple current, hash-verified execution sessions and an independent performance assessment satisfy the mastery policy."}else{"Artifact checks and self-reports do not establish independent capability runtime or user mastery. Multiple practices and a separately trusted performance assessment are required."}})
}
fn clean(s: &str) -> String {
    crate::intelligence::clean(s)
}
pub(crate) fn source_version(s: &Subject) -> String {
    digest(
        json!([
            s.logical_id,
            s.evidence["document"]["contentHash"],
            s.evidence["assessment"]["contentHash"],
            if s.evidence["document"]["contentHash"].is_null()
                && s.evidence["assessment"]["contentHash"].is_null()
            {
                s.evidence["revision"].clone()
            } else {
                Value::Null
            },
            s.summary
        ])
        .to_string()
        .as_bytes(),
    )
}
fn category(s: &Subject) -> String {
    if let Some(category) = &s.category {
        return category.clone();
    }
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let key = source_version(s);
    if let Ok(values) = cache.lock() {
        if let Some(value) = values.get(&key) {
            return value.clone();
        }
    }
    let value = analysis::classify(
        &s.name,
        &s.summary,
        s.evidence["document"]["text"].as_str().unwrap_or(""),
        &[],
    )
    .primary
    .unwrap_or("unclassified".into());
    if let Ok(mut values) = cache.lock() {
        if values.len() >= 512 {
            values.clear();
        }
        values.insert(key, value.clone());
    }
    value
}

fn profile(s: &Subject) -> Value {
    let cat = category(s);
    let kind = s.kind.as_str();
    json!({"capability_id":s.logical_id,"capability_type":s.kind,"capability_name":s.name,"source":s.evidence["source"],"current_version":source_version(s),"description":s.summary,"category":cat,"supported_tasks":"UNKNOWN until source-supported analysis","learning_objectives":[format!("Explain the documented purpose of {}",s.name),"Identify setup, permissions and limitations","Produce a small source-backed practice artifact"],"prerequisites":"Review source requirements and reviewed dependencies","required_environment":if kind=="mcp"{"Configured server and an independently authorized connection test"}else if kind=="plugin"{"Compatible Codex plugin format; review bundled tools, account and enablement requirements"}else{"Review SKILL.md and actual task requirements; a Skill is not a standalone executable"},"required_permissions":"UNKNOWN; no execution permission is inferred","estimated_difficulty":"Guided exercise; rules estimate, not measured proficiency","estimated_learning_time":"15–60 minutes; adjusted by your time budget","practice_scenarios":format!("A small {} artifact exercise based on the source",cat),"verification_methods":["Known local artifact validator","Explicit human review with limited scope"],"limitations":["Artifact checks do not prove that the selected Skill or tool was invoked","Accounts, Codex recognition and third-party runtime remain NOT VERIFIED"],"risk_level":if kind=="skill"{"REVIEW_REQUIRED"}else{"EXTERNAL_SETUP_REQUIRED"},"evidence_sources":s.evidence,"states":{"observed_installation":s.observed_status,"source_file_now":if s.evidence["document"]["availability"]=="READ_ONLY_SOURCE"{"PRESENT"}else{"UNKNOWN"},"codex_recognized":"NOT_TESTED","configured":if kind=="mcp"{"OBSERVED_CONFIGURATION_ONLY"}else{"UNKNOWN"},"connected":"NOT_TESTED","executed":"NOT_TESTED","capability_verified":"NOT_TESTED","user_mastery":"NOT_VERIFIED"}})
}
fn recipe(s: &Subject, nonce: &str) -> Value {
    let category = category(s);
    let recipe = match category.as_str() {
        "frontend" | "mobile" => "frontend",
        "analytics" | "spreadsheets" | "databases" => "analytics",
        "browser" | "testing" => "browser",
        "presentations" => "presentations",
        "video" => "video",
        _ => "report",
    };
    let nums: Vec<u32> = nonce
        .bytes()
        .take(3)
        .map(|n| u32::from(n) % 40 + 10)
        .collect();
    let total: u32 = nums.iter().sum();
    let (filename, goal, criteria) = match recipe {
        "frontend" => (
            "artifact.html",
            "Design a small responsive dashboard from the three supplied rows.",
            "Use main, nav, section, name=\"viewport\", and an @media rule. Include the challenge nonce in the source. Static source checks only; browser interactions are not verified.",
        ),
        "analytics" => (
            "artifact.json",
            "Calculate a three-row fixture summary.",
            "Return JSON with nonce, total (sum of the three fixture values) and row_count. Results are independently recomputed from this session's fixture.",
        ),
        "browser" => (
            "artifact.json",
            "Design three interaction checks for a local React page.",
            "Return JSON with nonce and cases containing selector, action and expected fields. This validates test-plan structure; it never accepts a claimed passed/exit_code log as execution evidence.",
        ),
        "presentations" => (
            "artifact.md",
            "Create a three-slide explanation of the supplied fixture.",
            "Use at least three '# ' slide headings and include the challenge nonce. Layout quality and rendering require separate review.",
        ),
        "video" => (
            "artifact.json",
            "Plan a short three-scene technical explanation.",
            "Return JSON with nonce and three scenes, each with startSeconds, endSeconds and description. This validates a storyboard only; no rendering tool is launched.",
        ),
        _ => (
            "artifact.md",
            "Prepare a source-backed report using the supplied fixture.",
            "Include '# Summary', '# Evidence' and '# Limitations', the exact statement 'Learning fixtures contain no private files.' and the challenge nonce.",
        ),
    };
    let external = s.kind == "plugin" || s.kind == "mcp";
    json!({"taskId":format!("artifact-v1:{recipe}"),"recipe":recipe,"category":category,"capabilityType":s.kind,"title":goal,"nonce":nonce,"filename":filename,"expectedTotal":total,"fixture":format!("Challenge: {nonce}\nLearning fixtures contain no private files.\nvalue\n{}\n{}\n{}\n",nums[0],nums[1],nums[2]),"instructions":format!("# {} practice\n\n{}\n\n{}\n\nUse only input.txt. Save {} in this directory. Do not run commands from downloaded sources.\n\nAccount/tool runtime is NOT VERIFIED. Artifact acceptance does not prove capability invocation or mastery.\nChallenge: {}",s.name,goal,criteria,filename,nonce),"requiredTools":if external{"Read-only source documentation; external tool startup requires separate authorization"}else{"Your editor or separately chosen tool; SkillVerse does not execute SKILL.md"},"preconditions":["Review the source and actual permissions","Use only the generated isolated input"],"expectedResult":criteria,"verificationMethod":"FIRST_PARTY_STATIC_ARTIFACT_V1","executionMode":"DRY_RUN_DEFAULT","externalRuntimeBlocked":external,"risk":"No account, MCP, plugin hook or third-party command is started"})
}
impl Database {
    async fn learning_profile(&self, s: &Subject) -> Result<Value> {
        let mut result = profile(s);
        let raw:Option<String>=sqlx::query_scalar("SELECT profile_json FROM ai_analysis_records WHERE logical_id=? ORDER BY created_at DESC LIMIT 1").bind(&s.logical_id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?;
        if let Some(raw) = raw {
            if let Ok(ai) = serde_json::from_str::<Value>(&raw) {
                if ai["evidence"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&s.evidence["id"]))
                {
                    if ai["supported_tasks"]
                        .as_array()
                        .is_some_and(|tasks| !tasks.is_empty())
                    {
                        result["supported_tasks"] = ai["supported_tasks"].clone();
                    }
                    result["ai_context"] = json!({"model":ai["model_version"],"analyzedAt":ai["analyzed_at"],"functions":ai["core_functions"],"useCases":ai["practical_use_cases"],"prerequisites":ai["prerequisites"],"independentVerification":false});
                }
            }
        }
        Ok(result)
    }

    pub async fn initialize_learning(&self) -> Result<Value> {
        let _guard = SERIAL.lock().await;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        if (9..=12).contains(&version) {
            return Ok(json!({"schema":version}));
        }
        if version != 8 {
            return Err("Complete source intelligence migration before learning migration.".into());
        }
        let root = std::path::Path::new("D:/PersonalProject_DevTools/SkillVerseLearningBackups");
        std::fs::create_dir_all(root).map_err(|_| "BACKUP_DIRECTORY")?;
        let backup = root.join(format!("schema8-{}.sqlite3", uuid::Uuid::new_v4()));
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_string_lossy().as_ref())
            .execute(&self.pool)
            .await
            .map_err(|_| "BACKUP_FAILED")?;
        let restored = backup.with_extension("restored.sqlite3");
        std::fs::copy(&backup, &restored).map_err(|_| "RESTORE_FAILED")?;
        let before = crate::capability_identity::database_fingerprint(&self.pool).await?;
        for path in [&backup, &restored] {
            let copy = Self::open_read_only(path.clone()).await?;
            copy.integrity_check().await?;
            if crate::capability_identity::database_fingerprint(&copy.pool).await? != before {
                return Err("BACKUP_FINGERPRINT_MISMATCH".into());
            }
            copy.pool.close().await;
        }
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        for statement in include_str!("../../migrations/0009_learning_verification.sql")
            .split(';')
            .filter(|s| !s.trim().is_empty())
        {
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(|_| "MIGRATION_ROLLED_BACK")?;
        }
        tx.commit().await.map_err(|_| "MIGRATION_COMMIT")?;
        self.integrity_check().await?;
        Ok(json!({"schema":9,"backup":backup,"restored":restored,"verified":true}))
    }
    async fn learning_event(&self, event: &str, details: Value) -> Result<()> {
        sqlx::query("INSERT INTO learning_events VALUES(?,?,?,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(event)
            .bind(details.to_string())
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(|_| "LEARNING_AUDIT")?;
        Ok(())
    }
    pub async fn learning_engine(&self, action: &str, args: Value) -> Result<Value> {
        if action == "initialize" {
            return self.initialize_learning().await;
        }
        let _guard = SERIAL.lock().await;
        let ready: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='learning_profiles')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|_| "DATABASE")?;
        if !ready {
            return if action == "state" {
                Ok(json!({"ready":false,"liveModel":"NOT VERIFIED"}))
            } else {
                Err("Enable the verified learning store first.".into())
            };
        }
        let subjects = if action == "plan" || action == "catalog" {
            self.intelligence_subjects().await?
        } else {
            self.intelligence_subjects_for(args["logicalId"].as_str(), args["entityId"].as_str())
                .await?
        };
        if action == "catalog" {
            return Ok(json!({"subjects":subjects,"liveModel":"NOT VERIFIED"}));
        }
        let selected = subjects.iter().find(|s| {
            Some(s.logical_id.as_str()) == args["logicalId"].as_str()
                || Some(s.entity_id.as_str()) == args["entityId"].as_str()
        });
        if action == "state" {
            let Some(s) = selected else {
                return Ok(
                    json!({"ready":true,"subjects":subjects,"profile":null,"paths":[],"sessions":[],"runs":[],"liveModel":"NOT VERIFIED"}),
                );
            };
            let rows:Vec<(String,String,String)>=sqlx::query_as("SELECT p.id,p.path_json,l.source_version FROM learning_paths p JOIN learning_profiles l ON l.id=p.profile_id WHERE l.logical_id=? ORDER BY p.created_at DESC LIMIT 50").bind(&s.logical_id).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
            let version = source_version(s);
            let mut profile_data = self.learning_profile(s).await?;
            let paths:Vec<Value>=rows.iter().map(|(id,raw,v)|json!({"id":id,"data":serde_json::from_str::<Value>(raw).unwrap_or(Value::Null),"stale":v!=&version})).collect();
            let sessions=sqlx::query("SELECT s.* FROM practice_sessions s JOIN learning_paths p ON p.id=s.path_id JOIN learning_profiles l ON l.id=p.profile_id WHERE l.logical_id=? ORDER BY s.created_at DESC LIMIT 100").bind(&s.logical_id).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
            let sessions:Vec<Value>=sessions.iter().map(|r|json!({"id":r.get::<String,_>("id"),"pathId":r.get::<String,_>("path_id"),"createdAt":r.get::<i64,_>("created_at"),"updatedAt":r.get::<i64,_>("updated_at"),"status":r.get::<String,_>("status"),"workspace":r.get::<String,_>("workspace"),"task":serde_json::from_str::<Value>(&r.get::<String,_>("task_json")).unwrap_or(Value::Null),"note":r.get::<String,_>("note"),"stale":r.get::<String,_>("source_version")!=version})).collect();
            let runs=sqlx::query("SELECT r.*,e.evidence_location,e.evidence_hash,e.evidence_type,e.trust FROM verification_runs r LEFT JOIN verification_evidence e ON e.verification_id=r.id WHERE r.logical_id=? ORDER BY r.started_at DESC LIMIT 100").bind(&s.logical_id).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
            let runs:Vec<Value>=runs.iter().map(|r|{let path=r.get::<Option<String>,_>("evidence_location");let hash=r.get::<Option<String>,_>("evidence_hash");let intact=path.as_ref().zip(hash.as_ref()).is_some_and(|(p,h)|artifacts::intact(p,h));let stale=r.get::<String,_>("capability_version")!=version;let original=r.get::<String,_>("status");json!({"id":r.get::<String,_>("id"),"sessionId":r.get::<String,_>("session_id"),"status":if stale{"STALE"}else if original=="PASSED"&&!intact{"INCONCLUSIVE"}else{&original},"recordedStatus":original,"logicalId":r.get::<String,_>("logical_id"),"capabilityVersion":r.get::<String,_>("capability_version"),"environment":serde_json::from_str::<Value>(&r.get::<String,_>("environment_json")).unwrap_or(Value::Null),"taskId":r.get::<String,_>("task_id"),"stale":stale,"evidenceIntact":intact,"evidenceLocation":path,"evidenceHash":hash,"evidenceType":r.get::<Option<String>,_>("evidence_type"),"trust":r.get::<Option<String>,_>("trust"),"method":r.get::<String,_>("test_method"),"failureReason":r.get::<String,_>("failure_reason"),"layers":serde_json::from_str::<Value>(&r.get::<String,_>("layers_json")).unwrap_or(Value::Null),"startedAt":r.get::<i64,_>("started_at"),"finishedAt":r.get::<i64,_>("finished_at")})}).collect();
            let mastery = mastery(&runs);
            profile_data["states"]["user_mastery"] = mastery["status"].clone();
            return Ok(
                json!({"ready":true,"profile":profile_data,"paths":paths,"sessions":sessions,"runs":runs,"liveModel":"NOT VERIFIED","mastery":mastery}),
            );
        }
        let s = selected.ok_or("Choose a current capability.")?;
        let version = source_version(s);
        if action == "plan" {
            let profile_data = self.learning_profile(s).await?;
            let goal = clean(
                args["goal"]
                    .as_str()
                    .unwrap_or("Understand and safely practice this capability."),
            );
            if goal.trim().is_empty() || goal.len() > 2000 {
                return Err("Describe a goal of at most 2000 characters.".into());
            }
            let experience = args["experience"].as_str().unwrap_or("beginner");
            if !["beginner", "experienced"].contains(&experience) {
                return Err("EXPERIENCE").map_err(String::from);
            }
            let budget = args["minutes"].as_i64().unwrap_or(30);
            if !(5..=480).contains(&budget) {
                return Err("TIME_BUDGET".into());
            }
            let dependencies:Vec<(String,String)>=sqlx::query_as("SELECT right_id,evidence_json FROM ai_capability_relations WHERE left_id=? AND kind='DEPENDS_ON' AND state='APPROVED' ORDER BY right_id").bind(&s.logical_id).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
            let prerequisites:Vec<_>=dependencies.iter().map(|(id,e)|{let other=subjects.iter().find(|s|&s.logical_id==id);json!({"logicalId":id,"available":other.is_some_and(|s|s.installed),"evidence":serde_json::from_str::<Value>(e).unwrap_or(Value::Null),"runtime":"NOT_VERIFIED"})}).collect();
            let previous_attempts:i64=sqlx::query_scalar("SELECT count(*) FROM practice_sessions s JOIN learning_paths p ON p.id=s.path_id JOIN learning_profiles l ON l.id=p.profile_id WHERE l.logical_id=? AND l.source_version=?").bind(&s.logical_id).bind(&version).fetch_one(&self.pool).await.map_err(|_|"DATABASE")?;
            let foundation: Option<String> =
                sqlx::query_scalar("SELECT status FROM learning_records WHERE entity_id=?")
                    .bind(&s.entity_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|_| "DATABASE")?;
            let key = digest(
                json!([
                    s.logical_id,
                    version,
                    goal,
                    experience,
                    budget,
                    previous_attempts,
                    foundation,
                    prerequisites,
                    "learning-v1"
                ])
                .to_string()
                .as_bytes(),
            );
            let cached: Option<(String, String)> =
                sqlx::query_as("SELECT id,path_json FROM learning_paths WHERE cache_key=?")
                    .bind(&key)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|_| "DATABASE")?;
            if let Some((id, data)) = cached {
                return Ok(
                    json!({"pathId":id,"path":serde_json::from_str::<Value>(&data).map_err(|_|"PATH_INVALID")?,"cached":true}),
                );
            }
            let profile_id = digest(format!("{}:{version}", s.logical_id).as_bytes());
            let path_id = uuid::Uuid::new_v4().to_string();
            let task = recipe(s, "Preview only; a fresh challenge is created per session");
            let conditions = [
                "Read the source and explain its limits; acknowledgement is self-reported",
                "Review observed setup and unknown permissions; installation does not complete this stage",
                "Create a practice workspace, perform the task and submit the actual artifact",
                "Inspect the validator scope, source version and hash-bound evidence; runtime stays independent",
                "Multiple practices plus independent performance evidence; cannot be obtained by clicking",
            ];
            let steps:Vec<_>=STAGES.iter().enumerate().map(|(i,name)|json!({"ordinal":i,"name":name,"completionCondition":conditions[i],"estimatedMinutes":(budget/5).max(1),"status":"NOT_STARTED","tools":task["requiredTools"]})).collect();
            let path = json!({"id":path_id,"logicalId":s.logical_id,"sourceVersion":version,"goal":goal,"experience":experience,"timeBudget":budget,"historySnapshot":{"previousAttempts":previous_attempts,"savedFoundation":foundation},"steps":steps,"practiceCountSuggested":if experience=="beginner"{3}else{2},"scope":if budget<20{"One compact artifact; larger goal remains incomplete"}else{"Guided source review and an artifact exercise"},"prerequisites":prerequisites,"practicePreview":task,"blockers":["Codex recognition, account permissions and runtime must be verified separately"],"nextStep":if previous_attempts>0{"Review previous evidence and choose a new challenge; do not repeat old completion claims."}else{"Explore the actual source, then create a practice workspace."},"executionAllowed":false});
            let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
            sqlx::query("INSERT INTO ai_subjects VALUES(?,?) ON CONFLICT DO NOTHING")
                .bind(&s.logical_id)
                .bind(&s.entity_id)
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
            sqlx::query("INSERT INTO learning_profiles VALUES(?,?,?,?,?) ON CONFLICT(logical_id,source_version) DO NOTHING").bind(&profile_id).bind(&s.logical_id).bind(&version).bind(profile_data.to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            sqlx::query("INSERT INTO learning_paths VALUES(?,?,?,?,?,?,?,?)")
                .bind(&path_id)
                .bind(&profile_id)
                .bind(&goal)
                .bind(experience)
                .bind(budget)
                .bind(key)
                .bind(path.to_string())
                .bind(now())
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
            for (i, name) in STAGES.iter().enumerate() {
                sqlx::query("INSERT INTO learning_steps VALUES(?,?,?,?)")
                    .bind(&path_id)
                    .bind(i as i64)
                    .bind(name)
                    .bind(conditions[i])
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            self.learning_event(
                "PATH_CREATED",
                json!({"pathId":path_id,"logicalId":s.logical_id,"version":version}),
            )
            .await?;
            return Ok(json!({"pathId":path_id,"path":path,"cached":false}));
        }
        if action == "create-session" {
            let path_id = args["pathId"].as_str().ok_or("PATH_ID")?;
            let path_version:Option<String>=sqlx::query_scalar("SELECT l.source_version FROM learning_paths p JOIN learning_profiles l ON l.id=p.profile_id WHERE p.id=? AND l.logical_id=?").bind(path_id).bind(&s.logical_id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?;
            if path_version.as_deref() != Some(&version) {
                return Err("PATH_STALE_OR_UNAVAILABLE".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let task = recipe(s, &uuid::Uuid::new_v4().to_string());
            let workspace = artifacts::workspace(&id, &task)?;
            sqlx::query("INSERT INTO practice_sessions VALUES(?,?,?,?,'READY',?,?,'',?,?)")
                .bind(&id)
                .bind(path_id)
                .bind(task["taskId"].as_str())
                .bind(&version)
                .bind(workspace.to_string_lossy().as_ref())
                .bind(task.to_string())
                .bind(now())
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(|_| "SESSION_SAVE_FAILED_WORKSPACE_RETAINED")?;
            self.learning_event("PRACTICE_CREATED", json!({"sessionId":id,"dryRun":true}))
                .await?;
            return Ok(json!({"sessionId":id,"workspace":workspace,"task":task,"status":"READY"}));
        }
        let session = args["sessionId"].as_str().ok_or("SESSION_ID")?;
        let row=sqlx::query("SELECT s.* FROM practice_sessions s JOIN learning_paths p ON p.id=s.path_id JOIN learning_profiles l ON l.id=p.profile_id WHERE s.id=? AND l.logical_id=?").bind(session).bind(&s.logical_id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?.ok_or("SESSION_UNAVAILABLE")?;
        if row.get::<String, _>("source_version") != version {
            return Err("SOURCE_VERSION_CHANGED_REGENERATE_PATH".into());
        }
        let status: String = row.get("status");
        let task: Value =
            serde_json::from_str(&row.get::<String, _>("task_json")).map_err(|_| "TASK_INVALID")?;
        if action == "load-artifact" || action == "save-artifact" {
            let workspace: String = row.get("workspace");
            let filename = task["filename"].as_str().ok_or("TASK_FILE")?;
            if action == "load-artifact" {
                let path = std::path::Path::new(&workspace).join(filename);
                if !path.exists() {
                    return Ok(json!({"body":"","hash":null}));
                }
                let bytes = artifacts::read_artifact(&workspace, filename)?;
                return Ok(
                    json!({"body":String::from_utf8(bytes.clone()).map_err(|_|"UTF8_ARTIFACT_REQUIRED")?,"hash":digest(&bytes)}),
                );
            }
            if !["READY", "IN_PROGRESS", "FAILED"].contains(&status.as_str()) {
                return Err("Start or resume practice before editing the artifact.".into());
            }
            let body = args["body"].as_str().ok_or("ARTIFACT_BODY")?;
            let hash = artifacts::save_artifact(
                &workspace,
                filename,
                body,
                args["expectedHash"].as_str(),
            )?;
            self.learning_event(
                "ARTIFACT_SAVED",
                json!({"sessionId":session,"hash":hash,"bytes":body.len()}),
            )
            .await?;
            return Ok(json!({"saved":true,"hash":hash}));
        }
        if action == "transition" {
            let next = args["status"].as_str().unwrap_or("");
            let allowed = matches!(
                (status.as_str(), next),
                ("READY", "IN_PROGRESS")
                    | ("IN_PROGRESS", "BLOCKED")
                    | ("BLOCKED", "IN_PROGRESS")
                    | ("IN_PROGRESS", "SUBMITTED")
                    | ("FAILED", "IN_PROGRESS")
            );
            if !allowed {
                return Err("INVALID_PRACTICE_TRANSITION_NO_MANUAL_VERIFIED_OR_MASTER".into());
            }
            let note = clean(args["note"].as_str().unwrap_or(""));
            if next == "SUBMITTED" && note.trim().is_empty() {
                return Err("Describe the actual attempt; self-report remains unverified.".into());
            }
            sqlx::query("UPDATE practice_sessions SET status=?,note=?,updated_at=? WHERE id=?")
                .bind(next)
                .bind(&note)
                .bind(now())
                .bind(session)
                .execute(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            self.learning_event(
                if next == "SUBMITTED" {
                    "SELF_REPORTED_SUBMISSION"
                } else {
                    "PRACTICE_TRANSITION"
                },
                json!({"sessionId":session,"status":next}),
            )
            .await?;
            return Ok(json!({"status":next,"trust":"SELF_REPORTED","mastery":"NOT_VERIFIED"}));
        }
        if action != "verify-artifact" && action != "manual-review" {
            return Err(
                "Unknown learning action; execution and mastery promotion are unavailable.".into(),
            );
        }
        if status != "SUBMITTED" || args["confirmed"] != true {
            return Err(
                "Submit your practice and explicitly approve the bounded evidence inspection."
                    .into(),
            );
        }
        let method = if action == "manual-review" {
            "HUMAN_REVIEW_LIMITED_SCOPE"
        } else {
            "FIRST_PARTY_STATIC_ARTIFACT_V1"
        };
        let started = now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let filename = task["filename"].as_str().ok_or("TASK_FILE")?;
        if args.get("artifactPath").is_some() || args.get("evidencePath").is_some() {
            return Err("ARBITRARY_EVIDENCE_PATH_REJECTED".into());
        }
        let fixture_bytes =
            artifacts::read_artifact(&row.get::<String, _>("workspace"), "input.txt")?;
        let fixture_hash = digest(&fixture_bytes);
        if fixture_bytes != task["fixture"].as_str().unwrap_or("").as_bytes() {
            return Err("FIXTURE_CHANGED_CREATE_NEW_SESSION".into());
        }
        let bytes = artifacts::read_artifact(&row.get::<String, _>("workspace"), filename)?;
        let hash = digest(&bytes);
        let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM verification_runs WHERE session_id=? AND test_method=? AND input_hash=?)").bind(session).bind(method).bind(&hash).fetch_one(&self.pool).await.map_err(|_|"DATABASE")?;
        if duplicate {
            return Err("DUPLICATE_EVIDENCE_SUBMISSION".into());
        }
        let manual = action == "manual-review";
        let result = if manual {
            Err(
                "Human review records an attestation, not independent automatic verification."
                    .to_string(),
            )
        } else {
            artifacts::check(&task, &bytes)
        };
        let run_status = if manual {
            "INCONCLUSIVE"
        } else if result.is_ok() {
            "PASSED"
        } else {
            "FAILED"
        };
        let mut failure = result.as_ref().err().cloned().unwrap_or_default();
        if manual {
            let note = clean(args["reviewNote"].as_str().unwrap_or(""));
            if note.trim().len() < 20 {
                return Err("Explain the human review in at least 20 characters; it remains an unverified attestation.".into());
            }
            failure.push_str(&format!(" Review note: {note}"));
        }
        let checks = result.unwrap_or_default();
        let evidence_path = artifacts::seal(&run_id, &bytes)?;
        let layers = json!({"files_or_configuration":if s.evidence["document"]["availability"]=="READ_ONLY_SOURCE"{"PASSED"}else{"NOT_TESTED"},"codex_recognition":"NOT_TESTED","dependencies":"NOT_TESTED","capability_runtime":"NOT_TESTED","tool_invocation":"NOT_TESTED","task_artifact":run_status,"user_performance":"INCONCLUSIVE","scope":"ARTIFACT_ONLY_NOT_CAPABILITY_EXECUTION","checks":checks});
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        sqlx::query("INSERT INTO verification_runs VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(&run_id).bind(session).bind(&s.logical_id).bind(&version).bind(json!({"os":std::env::consts::OS,"adapter":"native bounded data inspection","network":false,"commandsExecuted":false,"fixtureHash":fixture_hash}).to_string()).bind(task["taskId"].as_str()).bind(method).bind(started).bind(now()).bind(run_status).bind(&failure).bind(layers.to_string()).bind(&hash).execute(&mut *tx).await.map_err(|_|"VERIFICATION_COMMIT_FAILED_EVIDENCE_RETAINED")?;
        sqlx::query("INSERT INTO verification_evidence VALUES(?,?,?,?,?,?,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&run_id)
            .bind(if manual {
                "HUMAN_REVIEW"
            } else {
                "LOCAL_ARTIFACT"
            })
            .bind(evidence_path.to_string_lossy().as_ref())
            .bind(&hash)
            .bind(bytes.len() as i64)
            .bind(if manual {
                "HUMAN_REVIEW"
            } else {
                "LOCAL_VALIDATOR"
            })
            .execute(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
        sqlx::query("UPDATE practice_sessions SET status=?,updated_at=? WHERE id=?")
            .bind(if run_status == "PASSED" {
                "VERIFIED"
            } else if manual {
                "SUBMITTED"
            } else {
                "FAILED"
            })
            .bind(now())
            .bind(session)
            .execute(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
        sqlx::query("INSERT INTO learning_milestones VALUES(?,?,?,?,?,?,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&s.logical_id)
            .bind(&version)
            .bind("Practice")
            .bind(if run_status == "PASSED" {
                "ARTIFACT_VERIFIED"
            } else {
                "SELF_REPORTED"
            })
            .bind(&run_id)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
        sqlx::query("INSERT INTO learning_events VALUES(?,?,?,?)").bind(uuid::Uuid::new_v4().to_string()).bind("ARTIFACT_REVIEWED").bind(json!({"sessionId":session,"verificationId":run_id,"status":run_status,"hash":hash,"scope":"ARTIFACT_ONLY"}).to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
        tx.commit().await.map_err(|_| "DATABASE")?;
        Ok(
            json!({"verificationId":run_id,"status":run_status,"failureReason":failure,"layers":layers,"evidenceHash":hash,"evidenceLocation":evidence_path,"mastery":"NOT_VERIFIED"}),
        )
    }
}
