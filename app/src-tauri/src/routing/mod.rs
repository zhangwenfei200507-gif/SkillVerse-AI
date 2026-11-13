//! Project context and advice. The only effect beyond routing records is an
//! explicit, source-checked Mission Control draft handoff.
mod context;
mod decision;
mod evaluation;

use crate::{
    capability_identity::{database_fingerprint, database_fingerprint_on},
    db::Database,
    github::{
        analysis,
        types::{digest, now},
    },
    intelligence::clean,
    orchestration::{compatibility, security},
};
use decision::{Candidate, Intent, Ranked, Relation, ReviewRef, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

type Result<T> = std::result::Result<T, String>;
static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MIGRATION: &str = include_str!("../../migrations/0011_project_routing.sql");

fn task_understanding(
    goal: &str,
    intents: &[Intent],
    selected: &[String],
    relations: &[Relation],
) -> Value {
    let lower = goal.to_lowercase();
    let mut operations = vec!["analyze"];
    if [
        "fix",
        "modify",
        "implement",
        "修复",
        "修改",
        "开发",
        "优化",
        "optimiz",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        operations.push("modify");
    }
    if intents.contains(&Intent::BrowserTesting)
        || ["测试", "run tests"].iter().any(|s| lower.contains(s))
    {
        operations.push("test");
    }
    if ["delete", "push", "deploy", "删除", "推送", "部署"]
        .iter()
        .any(|s| lower.contains(s))
    {
        operations.push("external_change");
    }
    operations.push("report");
    let risky = operations.contains(&"modify") || operations.contains(&"external_change");
    json!({"schemaVersion":1,"goalSummary":if contaminated(goal){"Clarify the task without untrusted override instructions".into()}else{clean(goal).chars().take(300).collect::<String>()},
        "taskIntents":intents,"operations":operations,"requiredCapabilities":selected,
        "dependencies":relations.iter().filter(|r|r.state=="APPROVED"&&matches!(r.kind.as_str(),"DEPENDS_ON"|"PREREQUISITE")&&selected.contains(&r.left)&&selected.contains(&r.right)).map(|r|json!({"before":r.right,"after":r.left})).collect::<Vec<_>>(),
        "riskLevel":if risky{"HIGH"}else if selected.is_empty(){"LOW"}else{"MEDIUM"},"requiresApproval":true,
        "acceptanceCriteria":if operations.contains(&"test"){vec!["Validate the approved plan and exact source versions","Collect actual test evidence from a separately approved available runtime","Verify saved output integrity"]}else{vec!["Validate the approved plan and exact source versions","Collect only evidence within the adapter scope","Verify saved output integrity"]},
        "verification":"NOT_EXECUTED","source":"BOUNDED_RULES_WITH_VALIDATED_CAPABILITY_ADVICE"})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Proposal {
    id: String,
    schema_version: u8,
    context: context::Context,
    intents: Vec<Intent>,
    complexity: String,
    ranked: Vec<Ranked>,
    selected: Vec<String>,
    related: Vec<Relation>,
    reviewed_for_draft: Vec<ReviewRef>,
    missing: Vec<String>,
    workflow_preview: Vec<String>,
    semantic: Value,
    cached_context: bool,
    #[serde(default)]
    task: Option<Value>,
}

fn require_text<'a>(args: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= limit)
        .ok_or_else(|| format!("ROUTING_INVALID_{}", key.to_ascii_uppercase()))
}
fn confirmed(args: &Value) -> Result<()> {
    if args["confirmed"] != true {
        return Err("ROUTING_EXPLICIT_CONFIRMATION_REQUIRED".into());
    }
    Ok(())
}
fn uuid_arg(args: &Value, key: &str) -> Result<String> {
    uuid::Uuid::parse_str(require_text(args, key, 36)?)
        .map(|id| id.to_string())
        .map_err(|_| "ROUTING_INVALID_ID".into())
}
fn contaminated(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "ignore previous",
        "ignore all instructions",
        "system prompt",
        "reveal credentials",
        "bypass approval",
        "忽略之前",
        "泄露凭据",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}
fn digest_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut input = std::fs::File::open(path).map_err(|_| "ROUTING_BACKUP_UNREADABLE")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let len = input
            .read(&mut buffer)
            .map_err(|_| "ROUTING_BACKUP_UNREADABLE")?;
        if len == 0 {
            break;
        }
        hash.update(&buffer[..len]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl Database {
    async fn routing_schema(&self) -> Result<i64> {
        sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE".into())
    }

    async fn initialize_routing(&self) -> Result<Value> {
        let _serial = WRITES.lock().await;
        let schema = self.routing_schema().await?;
        if (11..=13).contains(&schema) {
            return Ok(json!({"schema":schema,"ready":true}));
        }
        if schema != 10 {
            return Err("ROUTING_REQUIRES_SCHEMA_10".into());
        }
        self.protected_ai_migration(10, 11, MIGRATION).await
    }

    pub(crate) async fn protected_ai_migration(
        &self,
        before_version: i64,
        after_version: i64,
        migration: &str,
    ) -> Result<Value> {
        self.integrity_check().await?;
        let folder = security::root().join("routing-backups");
        security::owned_dir(&folder)?;
        let backup = folder.join(format!(
            "schema{before_version}-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_string_lossy().as_ref())
            .execute(&self.pool)
            .await
            .map_err(|_| "ROUTING_BACKUP_FAILED")?;
        let restored = backup.with_extension("restored.sqlite3");
        std::fs::copy(&backup, &restored).map_err(|_| "ROUTING_RESTORE_FAILED")?;
        let sha256 = digest_file(&backup)?;
        if digest_file(&restored)? != sha256 {
            return Err("ROUTING_RESTORE_HASH_MISMATCH".into());
        }
        let backup_db = Self::open_read_only(backup.clone()).await?;
        backup_db.integrity_check().await?;
        let baseline = database_fingerprint(&backup_db.pool).await?;
        backup_db.pool.close().await;
        let restore_db = Self::open_read_only(restored.clone()).await?;
        restore_db.integrity_check().await?;
        if database_fingerprint(&restore_db.pool).await? != baseline {
            return Err("ROUTING_RESTORE_MISMATCH".into());
        }
        restore_db.pool.close().await;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| "ROUTING_MIGRATION_LOCK")?;
        if database_fingerprint_on(&mut tx).await? != baseline || baseline.schema != before_version
        {
            return Err("ROUTING_DATABASE_CHANGED_RETRY_BACKUP".into());
        }
        for statement in migration
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(|_| "ROUTING_MIGRATION_ROLLED_BACK")?;
        }
        let after = database_fingerprint_on(&mut tx).await?;
        let violations: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)")
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| "ROUTING_FOREIGN_KEY_CHECK")?;
        if after.schema != after_version
            || violations
            || baseline.tables.iter().any(|(name, old)| {
                name != "schema_migrations" && after.tables.get(name) != Some(old)
            })
        {
            return Err("ROUTING_PROTECTED_DATA_CHANGED".into());
        }
        tx.commit().await.map_err(|_| "ROUTING_MIGRATION_COMMIT")?;
        Ok(
            json!({"schema":after_version,"ready":true,"backup":backup,"restored":restored,"sha256":sha256,"backupVerified":true,
            "protectedTables":baseline.tables.len()-1}),
        )
    }

    async fn routing_status(&self) -> Result<Value> {
        let schema = self.routing_schema().await?;
        let settings = if schema >= 8 {
            self.ai_settings().await?
        } else {
            Default::default()
        };
        let recent: Vec<(String, String, i64, Option<String>)> = if (11..=13).contains(&schema) {
            sqlx::query_as("SELECT d.id,d.task_type,d.created_at,h.workflow_id FROM routing_decisions d LEFT JOIN routing_handoffs h ON h.decision_id=d.id ORDER BY d.created_at DESC,d.id LIMIT 15")
                .fetch_all(&self.pool).await.map_err(|_|"DATABASE")?
        } else {
            vec![]
        };
        let usage: Option<String> = if schema >= 8 {
            sqlx::query_scalar("SELECT error_code FROM ai_request_usage WHERE error_code IS NOT NULL ORDER BY rowid DESC LIMIT 1")
                .fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?
        } else {
            None
        };
        Ok(json!({"schema":schema,"ready":(11..=13).contains(&schema),
            "provider":{"provider":settings.provider,"model":settings.model,
                "connection":if settings.enabled && settings.provider != "unconfigured" && !settings.model.trim().is_empty(){"CONFIGURED_NOT_PROBED"}else{"MODEL_NOT_CONFIGURED"},
                "liveModel":"NOT_VERIFIED","estimatedCost":"UNKNOWN_UNTIL_AUTHORIZED_PROVIDER_PRICING",
                "dailyRequests":settings.daily_requests,"dailyTokens":settings.daily_tokens,
                "timeoutSeconds":settings.timeout_seconds,"lastError":usage.map(|s|clean(&s))},
            "history":recent.iter().map(|(id,kind,at,workflow)|json!({"id":id,"taskType":kind,"createdAt":at,"workflowId":workflow})).collect::<Vec<_>>() }))
    }

    async fn routing_candidates(&self) -> Result<Vec<Candidate>> {
        let subjects = self.intelligence_subjects().await?;
        let scan = self.load_codex_scan().await?;
        let mut disabled = BTreeSet::new();
        if let Some(scan) = &scan {
            disabled.extend(
                scan.plugins
                    .iter()
                    .filter(|p| p.enabled == Some(false))
                    .map(|p| p.id.as_str()),
            );
            disabled.extend(
                scan.skills
                    .iter()
                    .filter(|s| s.enabled == Some(false))
                    .map(|s| s.id.as_str()),
            );
            disabled.extend(
                scan.mcp_servers
                    .iter()
                    .filter(|s| !s.enabled)
                    .map(|s| s.id.as_str()),
            );
        }
        let classification: BTreeMap<String, (String, String)> =
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT entity_id,related_json,state FROM github_classifications",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|_| "DATABASE")?
            .into_iter()
            .map(|(id, related, state)| (id, (related, state)))
            .collect();
        let profiles: BTreeMap<String,Value>=sqlx::query_as::<_,(String,String)>(
            "SELECT logical_id,profile_json FROM ai_analysis_records a WHERE a.id=(SELECT id FROM ai_analysis_records b WHERE b.logical_id=a.logical_id ORDER BY created_at DESC,id DESC LIMIT 1)"
        ).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?.into_iter().filter_map(|(id,raw)|serde_json::from_str(&raw).ok().map(|p|(id,p))).collect();
        let history:BTreeMap<(String,String),u32>=sqlx::query_as::<_,(String,String,i64)>(
            "SELECT COALESCE(json_extract(n.node_json,'$.capabilityId'),'review:'||json_extract(n.node_json,'$.params.profileId')),json_extract(n.node_json,'$.capabilityVersion'),count(*)
             FROM execution_steps s JOIN execution_runs r ON s.execution_id=r.id
             JOIN workflow_nodes n ON n.workflow_id=r.workflow_id AND n.revision=r.revision AND n.node_id=s.node_id
             JOIN execution_evidence e ON e.execution_id=s.execution_id AND e.node_id=s.node_id
             WHERE s.status='SUCCEEDED' AND r.status='SUCCEEDED' AND r.mode<>'DRY_RUN'
             AND (json_extract(n.node_json,'$.capabilityId') IS NOT NULL OR json_extract(n.node_json,'$.adapter') IN ('skill.reviewed','plugin.reviewed','mcp.reviewed'))
             AND json_extract(n.node_json,'$.capabilityVersion') IS NOT NULL
             GROUP BY 1,2"
        ).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?.into_iter().map(|(id,version,count)|((id,version),count.min(u32::MAX as i64) as u32)).collect();
        let catalog_keys = analysis::category_keys();
        let mut candidates = Vec::new();
        for subject in subjects {
            if !matches!(subject.kind.as_str(), "skill" | "plugin" | "mcp") {
                continue;
            }
            let version = crate::learning::source_version(&subject);
            let profile = profiles.get(&subject.logical_id).filter(|p| {
                p["evidence"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&subject.evidence["id"]))
            });
            let mut purpose = format!(
                "{} {}",
                subject.name,
                subject.summary.chars().take(600).collect::<String>()
            );
            if let Some(p) = profile {
                for item in ["core_functions", "supported_tasks"]
                    .iter()
                    .filter_map(|key| p[*key].as_array())
                    .flatten()
                    .filter_map(Value::as_str)
                    .take(8)
                {
                    purpose.push(' ');
                    purpose.extend(item.chars().take(160));
                }
            }
            let related = classification
                .get(&subject.entity_id)
                .and_then(|(json, _)| serde_json::from_str::<Vec<String>>(json).ok())
                .unwrap_or_default();
            let secondary_categories = related
                .into_iter()
                .filter(|key| catalog_keys.contains(key) && subject.category.as_ref() != Some(key))
                .take(5)
                .collect();
            let suspect = contaminated(&purpose)
                || contaminated(subject.evidence["document"]["text"].as_str().unwrap_or(""));
            let status = if suspect {
                "UNTRUSTED_METADATA"
            } else if disabled.contains(subject.entity_id.as_str()) {
                "DISABLED"
            } else if subject.kind == "mcp" && subject.observed_status == "CONFIGURED_UNVERIFIED" {
                "CONFIGURED_UNVERIFIED"
            } else if subject.installed {
                "INSTALLATION_OBSERVED"
            } else {
                "DISCOVERY_ONLY"
            };
            let mut missing = vec![
                match status {
                    "UNTRUSTED_METADATA" => "Source text requires review before routing",
                    "DISABLED" => "Capability is disabled in the saved scan",
                    "CONFIGURED_UNVERIFIED" => "Current MCP handshake and exact tool schema",
                    "INSTALLATION_OBSERVED" => {
                        "Current Codex recognition and scoped execution adapter"
                    }
                    _ => "Installation and exact runtime compatibility",
                }
                .to_string(),
            ];
            if let Some(prerequisites) = profile.and_then(|p| p["prerequisites"].as_array()) {
                if !prerequisites.is_empty() {
                    missing.push("Review the source profile's declared prerequisites".into());
                }
            }
            let successes = history
                .get(&(subject.logical_id.clone(), version.clone()))
                .copied()
                .unwrap_or(0);
            let classification_state = if subject.manual {
                "USER_REVIEWED".into()
            } else if subject.category.is_none() {
                "PENDING_REVIEW".into()
            } else {
                classification
                    .get(&subject.entity_id)
                    .map(|(_, state)| state.clone())
                    .unwrap_or("SOURCE_CLASSIFIED".into())
            };
            candidates.push(Candidate {
                id: subject.logical_id,
                version,
                name: clean(&subject.name).chars().take(110).collect(),
                kind: subject.kind,
                category: subject.category,
                secondary_categories,
                classification: classification_state,
                purpose,
                installation: subject.observed_status,
                recognition: "NOT_VERIFIED".into(),
                runtime: "NOT_VERIFIED".into(),
                availability: status.into(),
                risk: "EXTERNAL_SETUP_REQUIRED".into(),
                cost: "UNKNOWN".into(),
                prior_successes: successes,
                missing,
                review: None,
            });
        }
        let listed = compatibility::list()?;
        for raw in listed["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .take(150)
        {
            let (Some(id), Some(version), Some(kind), Some(name)) = (
                raw["id"].as_str(),
                raw["digest"].as_str(),
                raw["kind"].as_str(),
                raw["name"].as_str(),
            ) else {
                continue;
            };
            let mut policy_ready =
                raw["policyReady"] == true && raw["enabled"] == true && !contaminated(name);
            let profile = compatibility::load(id, version)?;
            let description = if policy_ready
                && kind != "mcp"
                && profile.files.contains_key(&profile.component)
            {
                let path = Path::new(&profile.root).join(&profile.component);
                security::read(&path, 131072)
                    .ok()
                    .filter(|bytes| digest(bytes) == profile.files[&profile.component])
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .and_then(|text| {
                        let header = text.strip_prefix("---")?.split("\n---").next()?;
                        header
                            .lines()
                            .find_map(|line| line.strip_prefix("description:"))
                            .map(|line| line.chars().take(400).collect::<String>())
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            policy_ready &= !contaminated(&description);
            let text = format!(
                "{} {} {}",
                name,
                if kind == "mcp" {
                    "public documentation research Codex"
                } else {
                    &profile.component
                },
                description
            );
            let reviewed_id = format!("review:{id}");
            let successes = history
                .get(&(reviewed_id.clone(), version.into()))
                .copied()
                .unwrap_or(0);
            candidates.push(Candidate {
                id: reviewed_id,
                version: version.into(),
                name: clean(name).chars().take(110).collect(),
                kind: kind.into(),
                category: None,
                secondary_categories: vec![],
                classification: "REVIEWED_SOURCE".into(),
                purpose: text,
                installation: "REVIEWED_SOURCE".into(),
                recognition: "NOT_VERIFIED".into(),
                runtime: "NOT_VERIFIED".into(),
                availability: if policy_ready {
                    "SCOPE_REVIEW_AVAILABLE"
                } else if raw["enabled"] != true {
                    "DISABLED"
                } else {
                    "REVIEW_BLOCKED"
                }
                .into(),
                risk: "SCOPED_APPROVAL_REQUIRED".into(),
                cost: if kind == "mcp" {
                    "PUBLIC_READ_ONLY_NETWORK"
                } else {
                    "ONE_BOUNDED_CODEX_TURN"
                }
                .into(),
                prior_successes: successes,
                missing: vec![
                    if policy_ready {
                        "Fresh recognition or handshake after approval"
                    } else {
                        "Source disabled or review policy no longer passes"
                    }
                    .into(),
                ],
                review: Some(ReviewRef {
                    id: id.into(),
                    digest: version.into(),
                }),
            });
        }
        Ok(candidates)
    }

    async fn routing_relations(&self) -> Result<Vec<Relation>> {
        let rows=sqlx::query("SELECT left_id,right_id,kind,state,evidence_json FROM ai_capability_relations ORDER BY left_id,right_id,kind LIMIT 2000")
            .fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
        Ok(rows
            .iter()
            .map(|row| {
                let evidence: Value =
                    serde_json::from_str(row.get("evidence_json")).unwrap_or(Value::Null);
                Relation {
                    left: row.get("left_id"),
                    right: row.get("right_id"),
                    kind: row.get("kind"),
                    state: row.get("state"),
                    source: if evidence["reviewedBy"] == "user" {
                        "USER_REVIEW".into()
                    } else {
                        "SOURCE_EVIDENCE".into()
                    },
                    reason: clean(
                        evidence["reason"]
                            .as_str()
                            .unwrap_or("Relationship evidence unavailable"),
                    )
                    .chars()
                    .take(300)
                    .collect(),
                    confidence: evidence["confidence"]
                        .as_f64()
                        .filter(|x| (0.0..=1.0).contains(x)),
                }
            })
            .collect())
    }

    async fn routing_analyze(&self, args: &Value) -> Result<Value> {
        confirmed(args)?;
        let goal = require_text(args, "goal", 2000)?;
        let selection = context::select(require_text(args, "projectRoot", 2048)?)?;
        let cached: Option<String> = if args["refresh"] == true {
            None
        } else {
            sqlx::query_scalar(
                "SELECT context_json FROM routing_project_contexts WHERE root_key=? AND stamp=?",
            )
            .bind(&selection.root_key)
            .bind(&selection.stamp)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| "DATABASE")?
        };
        let context: context::Context = match cached.as_deref() {
            Some(raw) => serde_json::from_str(raw).map_err(|_| "ROUTING_CACHE_INVALID")?,
            None => context::snapshot(&selection),
        };
        let intents = decision::intents(goal);
        let task_type = intents[0].key();
        let feedback:BTreeMap<String,i32>=sqlx::query_as::<_,(String,i32)>(
            "SELECT f.chosen_id,CASE WHEN sum(f.rating='HELPFUL')>=2 THEN 2 WHEN sum(f.rating='INACCURATE')>=2 THEN -2 ELSE 0 END
             FROM routing_feedback f JOIN routing_decisions d ON d.id=f.decision_id
             WHERE d.task_type=? AND f.chosen_id IS NOT NULL AND f.failure_class IN('NONE','SELECTION') GROUP BY f.chosen_id"
        ).bind(task_type).fetch_all(&self.pool).await.map_err(|_|"DATABASE")?.into_iter().filter(|(_,n)|*n!=0).collect();
        let ranked = decision::rank(
            self.routing_candidates().await?,
            &intents,
            &context,
            &feedback,
        );
        let relations = self.routing_relations().await?;
        let (mut selected, mut semantic) =
            decision::hybrid(None, &ranked, &intents, &relations).await;
        if !args["modelRequest"].is_null() {
            if self.routing_schema().await? < 12 {
                return Err("MODEL_REGISTRY_NOT_INITIALIZED".into());
            }
            let input = json!({"schema_version":1,"task":if contaminated(goal){"Untrusted instructions require clarification".into()}else{clean(goal).chars().take(300).collect::<String>()},"project":{"frameworks":context.frameworks,"languages":context.languages,"testTools":context.test_tools},"intents":intents,"preferred":selected,
                "candidates":decision::model_candidates(&ranked),
                "relationships":relations.iter().filter(|r|r.state=="APPROVED").take(40).collect::<Vec<_>>()});
            match self
                .generate_model_advice(&args["modelRequest"], &input, true, |value| {
                    decision::validate_advice(value.clone(), &ranked, &relations).map(|_| ())
                })
                .await
            {
                Ok(mut receipt) => {
                    let value = receipt.as_object_mut().unwrap().remove("value").unwrap();
                    let (ids, confidence) =
                        decision::checked_selections(value, &ranked, &relations)?;
                    selected = ids;
                    let verified = receipt["provider"]
                        .as_str()
                        .is_some_and(|id| id == "phase13-openai" || id == "phase13-deepseek");
                    semantic = json!({"mode":if receipt["source"]=="MOCK"{"MOCK_STRUCTURED_ADVICE"}else{"LIVE_ADAPTER_ADVICE"},"status":"VALIDATED_ADVICE","confidence":confidence,"receipt":receipt,"liveModel":if verified{"MODEL_RESPONSE_VERIFIED"}else{"NOT_VERIFIED"}});
                }
                Err(error) if error == "CANCELLED" => return Err(error),
                Err(error) => {
                    semantic =
                        json!({"mode":"RULES_FALLBACK","status":error,"liveModel":"NOT_VERIFIED"});
                }
            }
        }
        let (reviewed, blocked) = decision::eligible_reviews(&selected, &ranked, &relations);
        let mut missing = Vec::new();
        if intents == [Intent::Unclear] {
            missing.push("Specify the expected artifact or action to narrow the route".into());
        }
        if intents.contains(&Intent::BrowserTesting) {
            missing.push("Browser execution remains unavailable without a separately verified browser runtime and approval".into());
        }
        for id in blocked {
            missing.push(format!("Reviewed prerequisites are unresolved for {id}"));
        }
        let mut related: Vec<_> = relations
            .into_iter()
            .filter(|rel| selected.contains(&rel.left) || selected.contains(&rel.right))
            .take(24)
            .collect();
        for pair in selected.windows(2) {
            if related.iter().any(|r| {
                (r.left == pair[0] && r.right == pair[1])
                    || (r.left == pair[1] && r.right == pair[0])
            }) {
                continue;
            }
            if let (Some(left), Some(right)) = (
                ranked.iter().find(|r| r.candidate.id == pair[0]),
                ranked.iter().find(|r| r.candidate.id == pair[1]),
            ) {
                if left.facets != right.facets {
                    related.push(Relation{left:pair[0].clone(),right:pair[1].clone(),kind:"COMPLEMENTARY".into(),
                        state:"UNVERIFIED_RELATION".into(),source:"TASK_FACET_RULE".into(),
                        reason:"The documented purposes match different task facets. Review the sources before confirming a relationship.".into(),confidence:None});
                }
            }
        }
        for rel in related.iter().filter(|r| {
            r.state == "APPROVED" && matches!(r.kind.as_str(), "DEPENDS_ON" | "PREREQUISITE")
        }) {
            if selected.contains(&rel.left) && !selected.contains(&rel.right) {
                missing.push(format!(
                    "Reviewed prerequisite {} needs separate scope review",
                    rel.right
                ));
            }
        }
        let mut workflow_preview = vec![
            "Inspect selected project metadata".into(),
            "Review capability match and constraints".into(),
        ];
        for r in &reviewed {
            if let Some(c) = ranked
                .iter()
                .find(|c| c.candidate.review.as_ref() == Some(r))
            {
                workflow_preview.push(c.candidate.name.clone());
            }
        }
        workflow_preview.extend(["Write a scoped report".into(), "Verify report hash".into()]);
        let task = task_understanding(goal, &intents, &selected, &related);
        if task["riskLevel"] == "HIGH" {
            missing.push("Project edits or external changes need a separately supported adapter and exact execution approval; this recommendation grants neither.".into());
        }
        let proposal = Proposal {
            id: uuid::Uuid::new_v4().to_string(),
            schema_version: SCHEMA_VERSION,
            context,
            intents: intents.clone(),
            complexity: if intents.len() > 2
                || task["operations"]
                    .as_array()
                    .is_some_and(|ops| ops.len() > 3)
            {
                "COMPLEX"
            } else if intents.len() > 1 {
                "MEDIUM"
            } else {
                "SIMPLE"
            }
            .into(),
            ranked,
            selected,
            related,
            reviewed_for_draft: reviewed,
            missing,
            workflow_preview,
            semantic,
            cached_context: cached.is_some(),
            task: Some(task),
        };
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        sqlx::query("INSERT INTO routing_project_contexts VALUES(?,?,?,?,?) ON CONFLICT(root_key) DO UPDATE SET fingerprint=excluded.fingerprint,stamp=excluded.stamp,context_json=excluded.context_json,updated_at=excluded.updated_at")
            .bind(&selection.root_key).bind(&proposal.context.fingerprint).bind(&selection.stamp).bind(json!(proposal.context).to_string()).bind(now())
            .execute(&mut *tx).await.map_err(|_|"DATABASE")?;
        let document = json!(proposal);
        sqlx::query("INSERT INTO routing_decisions VALUES(?,?,?,?,?,?,?)")
            .bind(&proposal.id)
            .bind(&selection.root_key)
            .bind(&proposal.context.fingerprint)
            .bind(digest(goal.as_bytes()))
            .bind(task_type)
            .bind(document.to_string())
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
        tx.commit().await.map_err(|_| "DATABASE")?;
        Ok(document)
    }

    async fn routing_handoff(&self, args: &Value) -> Result<Value> {
        confirmed(args)?;
        let first_party = match args.get("firstPartyOnly") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            _ => return Err("ROUTING_INVALID_EXECUTION_PROFILE".into()),
        };
        let id = uuid_arg(args, "decisionId")?;
        let goal = require_text(args, "goal", 2000)?;
        let selection = context::select(require_text(args, "projectRoot", 2048)?)?;
        let row=sqlx::query("SELECT root_key,context_fingerprint,goal_digest,decision_json FROM routing_decisions WHERE id=?")
            .bind(&id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?.ok_or("ROUTING_DECISION_NOT_FOUND")?;
        if row.get::<&str, _>("root_key") != selection.root_key
            || row.get::<&str, _>("goal_digest") != digest(goal.as_bytes())
            || row.get::<&str, _>("context_fingerprint")
                != context::snapshot(&selection).fingerprint
        {
            return Err("ROUTING_GOAL_OR_PROJECT_CHANGED_REANALYZE".into());
        }
        let proposal: Proposal = serde_json::from_str(row.get("decision_json"))
            .map_err(|_| "ROUTING_DECISION_INVALID")?;
        if proposal.schema_version != SCHEMA_VERSION {
            return Err("ROUTING_DECISION_SCHEMA_CHANGED".into());
        }
        let candidates = self.routing_candidates().await?;
        let current_ranked: Vec<_> = proposal
            .ranked
            .iter()
            .filter_map(|r| {
                candidates
                    .iter()
                    .find(|c| {
                        c.id == r.candidate.id
                            && c.version == r.candidate.version
                            && (!first_party || c.availability == r.candidate.availability)
                    })
                    .map(|current| Ranked {
                        candidate: current.clone(),
                        score: r.score,
                        facets: r.facets.clone(),
                        reasons: r.reasons.clone(),
                    })
            })
            .collect();
        if !proposal
            .selected
            .iter()
            .all(|id| current_ranked.iter().any(|r| r.candidate.id == *id))
        {
            return Err("ROUTING_CAPABILITY_CHANGED_REANALYZE".into());
        }
        let current_relations = self.routing_relations().await?;
        let (reviews, _) =
            decision::eligible_reviews(&proposal.selected, &current_ranked, &current_relations);
        if reviews != proposal.reviewed_for_draft {
            return Err("ROUTING_SCOPE_CHANGED_REANALYZE".into());
        }
        if !first_party {
            for review in &reviews {
                compatibility::check(&compatibility::load(&review.id, &review.digest)?)?;
            }
        }
        let mut planning =
            json!({"mode":proposal.semantic["mode"],"status":proposal.semantic["status"]});
        if first_party && proposal.semantic["mode"] == "LIVE_ADAPTER_ADVICE" {
            let receipt = &proposal.semantic["receipt"];
            planning["cached"] = json!(receipt["cacheHit"] == true);
            if receipt["cacheHit"] != true
                && proposal.semantic["liveModel"] == "MODEL_RESPONSE_VERIFIED"
            {
                let row = sqlx::query(
                    "SELECT s.request_id,s.provider_id,s.model_id,s.reported_model,s.reserved_microusd,s.settled_microusd
                     FROM ai_live_spend s JOIN ai_model_calls c ON c.id=s.request_id
                     WHERE s.request_id=? AND s.provider_id=? AND c.model_id=? AND s.model_id=?
                     AND s.purpose='planning'
                     AND c.status='SUCCEEDED'",
                )
                .bind(receipt["requestId"].as_str()).bind(receipt["provider"].as_str())
                .bind(receipt["modelId"].as_str()).bind(receipt["actualModelId"].as_str())
                .fetch_optional(&self.pool).await
                .map_err(|_| "DATABASE")?.ok_or("ROUTING_PLANNING_RECEIPT_UNVERIFIED")?;
                planning["ledger"] = json!({
                    "requestId":row.get::<String,_>("request_id"),
                    "provider":row.get::<String,_>("provider_id"),
                    "model":row.get::<Option<String>,_>("reported_model").unwrap_or_else(|| row.get("model_id")),
                    "reservedMicrousd":row.get::<i64,_>("reserved_microusd"),
                    "settledMicrousd":row.get::<Option<i64>,_>("settled_microusd")
                });
            }
        }
        self.save_routing_draft(&id,clean(goal),selection.path.to_string_lossy().into_owned(),json!({
            "decisionId":id,"schemaVersion":SCHEMA_VERSION,"projectFingerprint":proposal.context.fingerprint,
            "planning":planning,
            "intents":proposal.intents,"selected":proposal.selected,"missing":proposal.missing,
            "recommendations":proposal.ranked.iter().filter(|r|proposal.selected.contains(&r.candidate.id))
                .map(|r|json!({"id":r.candidate.id,"name":r.candidate.name,"score":r.score,"reasons":r.reasons,
                    "availability":r.candidate.availability,"runtime":"NOT_VERIFIED"})).collect::<Vec<_>>()
        }),reviews.iter().map(|review|json!(review)).collect(),first_party).await
    }

    async fn routing_replace(&self, args: &Value) -> Result<Value> {
        confirmed(args)?;
        let id = uuid_arg(args, "decisionId")?;
        let goal = require_text(args, "goal", 2000)?;
        let selection = context::select(require_text(args, "projectRoot", 2048)?)?;
        let row=sqlx::query("SELECT root_key,context_fingerprint,goal_digest,decision_json,task_type FROM routing_decisions WHERE id=?").bind(&id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?.ok_or("ROUTING_DECISION_NOT_FOUND")?;
        if row.get::<&str, _>("root_key") != selection.root_key
            || row.get::<&str, _>("goal_digest") != digest(goal.as_bytes())
            || row.get::<&str, _>("context_fingerprint")
                != context::snapshot(&selection).fingerprint
        {
            return Err("ROUTING_GOAL_OR_PROJECT_CHANGED_REANALYZE".into());
        }
        let mut proposal: Proposal = serde_json::from_str(row.get("decision_json"))
            .map_err(|_| "ROUTING_DECISION_INVALID")?;
        let ids: Vec<String> = serde_json::from_value(args["selected"].clone())
            .map_err(|_| "ROUTING_INVALID_SELECTION")?;
        if ids.len() > 3 {
            return Err("ROUTING_INVALID_SELECTION".into());
        }
        let relations = self.routing_relations().await?;
        if !ids.is_empty() {
            let advice = json!({"schema_version":1,"confidence":0.0,"selections":ids.iter().map(|id|json!({"id":id,"version":proposal.ranked.iter().find(|r|r.candidate.id==*id).map(|r|r.candidate.version.clone()),"reason":"PURPOSE_MATCH"})).collect::<Vec<_>>()});
            decision::validate_advice(advice, &proposal.ranked, &relations)?;
        }
        let current = self.routing_candidates().await?;
        for id in &ids {
            let old = proposal
                .ranked
                .iter()
                .find(|r| r.candidate.id == *id)
                .ok_or("ROUTING_INVALID_SELECTION")?;
            if !current.iter().any(|c| {
                c.id == *id
                    && c.version == old.candidate.version
                    && c.availability == old.candidate.availability
            }) {
                return Err("ROUTING_CAPABILITY_CHANGED_REANALYZE".into());
            }
        }
        let (reviews, blocked) = decision::eligible_reviews(&ids, &proposal.ranked, &relations);
        if !blocked.is_empty() {
            return Err("ROUTING_PREREQUISITES_UNRESOLVED".into());
        }
        proposal.id = uuid::Uuid::new_v4().to_string();
        proposal.selected = ids;
        proposal.reviewed_for_draft = reviews;
        proposal.semantic = json!({"mode":"USER_SELECTION","status":"EXPLICIT_REPLACEMENT","liveModel":"NOT_VERIFIED"});
        proposal.task = Some(task_understanding(
            goal,
            &proposal.intents,
            &proposal.selected,
            &relations,
        ));
        proposal.workflow_preview = vec![
            "Inspect selected project metadata".into(),
            "Review capability match and constraints".into(),
        ];
        proposal.workflow_preview.extend(
            proposal
                .ranked
                .iter()
                .filter(|r| {
                    proposal.selected.contains(&r.candidate.id) && r.candidate.review.is_some()
                })
                .map(|r| r.candidate.name.clone()),
        );
        proposal
            .workflow_preview
            .extend(["Write a scoped report".into(), "Verify report hash".into()]);
        let value = json!(proposal);
        sqlx::query("INSERT INTO routing_decisions VALUES(?,?,?,?,?,?,?)")
            .bind(&proposal.id)
            .bind(&selection.root_key)
            .bind(row.get::<&str, _>("context_fingerprint"))
            .bind(row.get::<&str, _>("goal_digest"))
            .bind(row.get::<&str, _>("task_type"))
            .bind(value.to_string())
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        Ok(value)
    }

    async fn routing_feedback(&self, args: &Value) -> Result<Value> {
        let id = uuid_arg(args, "decisionId")?;
        let rating = require_text(args, "rating", 32)?;
        if ![
            "HELPFUL",
            "INACCURATE",
            "UNAVAILABLE",
            "BETTER_ALTERNATIVE",
            "TOO_COMPLEX",
        ]
        .contains(&rating)
        {
            return Err("ROUTING_INVALID_RATING".into());
        }
        let failure = args["failureClass"].as_str().unwrap_or("NONE");
        if ![
            "NONE",
            "TOOL",
            "SETUP",
            "NETWORK",
            "SELECTION",
            "CANCELLED",
            "DENIED",
            "UNKNOWN",
        ]
        .contains(&failure)
        {
            return Err("ROUTING_INVALID_FAILURE_CLASS".into());
        }
        let raw: Option<String> =
            sqlx::query_scalar("SELECT decision_json FROM routing_decisions WHERE id=?")
                .bind(&id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
        let proposal: Proposal = serde_json::from_str(&raw.ok_or("ROUTING_DECISION_NOT_FOUND")?)
            .map_err(|_| "ROUTING_DECISION_INVALID")?;
        let chosen = args["chosenId"]
            .as_str()
            .or_else(|| proposal.selected.first().map(String::as_str));
        if chosen.is_some_and(|choice| !proposal.ranked.iter().any(|r| r.candidate.id == choice)) {
            return Err("ROUTING_UNKNOWN_CHOICE".into());
        }
        let run:Option<(String,String)>=sqlx::query_as("SELECT r.id,r.status FROM routing_handoffs h JOIN execution_runs r ON r.workflow_id=h.workflow_id WHERE h.decision_id=? AND r.mode<>'DRY_RUN' ORDER BY r.created_at DESC,r.id DESC LIMIT 1")
            .bind(&id).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?;
        sqlx::query("INSERT INTO routing_feedback VALUES(?,?,?,?,?,?,?) ON CONFLICT(decision_id) DO UPDATE SET rating=excluded.rating,failure_class=excluded.failure_class,chosen_id=excluded.chosen_id,execution_id=excluded.execution_id,execution_status=excluded.execution_status,updated_at=excluded.updated_at")
            .bind(&id).bind(rating).bind(failure).bind(chosen).bind(run.as_ref().map(|(id,_)|id)).bind(run.as_ref().map(|(_,s)|s)).bind(now())
            .execute(&self.pool).await.map_err(|_|"DATABASE")?;
        Ok(json!({"saved":true,"observedExecutionStatus":run.map(|(_,status)|status)}))
    }

    pub(crate) async fn project_routing(&self, action: &str, args: Value) -> Result<Value> {
        if action == "routing-status" {
            return self.routing_status().await;
        }
        if action == "routing-initialize" {
            confirmed(&args)?;
            return self.initialize_routing().await;
        }
        if !(11..=13).contains(&self.routing_schema().await?) {
            return Err("ROUTING_STORAGE_NOT_ENABLED".into());
        }
        match action {
            "routing-analyze" => self.routing_analyze(&args).await,
            "routing-handoff" => self.routing_handoff(&args).await,
            "routing-replace" => self.routing_replace(&args).await,
            "routing-feedback" => self.routing_feedback(&args).await,
            "routing-get" => {
                let id = uuid_arg(&args, "decisionId")?;
                let raw: Option<String> =
                    sqlx::query_scalar("SELECT decision_json FROM routing_decisions WHERE id=?")
                        .bind(&id)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(|_| "DATABASE")?;
                serde_json::from_str(&raw.ok_or("ROUTING_DECISION_NOT_FOUND")?)
                    .map_err(|_| "ROUTING_DECISION_INVALID".into())
            }
            "routing-evaluate" => evaluation::run(self).await,
            _ => Err("UNKNOWN_ROUTING_ACTION".into()),
        }
    }
}

#[cfg(test)]
mod tests;
