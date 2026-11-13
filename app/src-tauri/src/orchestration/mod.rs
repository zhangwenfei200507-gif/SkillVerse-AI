//! Durable, approval-bound workflow orchestration. External text is never code.
mod adapters;
mod codex_skill;
pub(crate) mod compatibility;
mod diagnostics;
mod mcp;
mod planner;
mod runner;
mod runtime;
pub(crate) mod security;
pub(crate) use security::ordinary as is_ordinary_path;
#[cfg(test)]
pub(crate) mod tests;
pub mod transports;
pub mod types;
use crate::{
    db::Database,
    github::types::{digest, now},
};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};
use types::{Node, Plan, Result};
fn step_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "id":row.get::<String,_>("id"), "nodeId":row.get::<String,_>("node_id"),
        "attempt":row.get::<i64,_>("attempt"), "status":row.get::<String,_>("status"),
        "startedAt":row.get::<i64,_>("started_at"), "finishedAt":row.get::<Option<i64>,_>("finished_at"),
        "output":row.get::<Option<String>,_>("output_json").and_then(|s|serde_json::from_str::<Value>(&s).ok()),
        "inputDigest":row.get::<String,_>("input_digest"), "outputDigest":row.get::<Option<String>,_>("output_digest"),
        "errorType":row.get::<String,_>("error_type"), "sideEffect":row.get::<String,_>("side_effect"),
    })
}
static MUTATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
type Controls = Mutex<BTreeMap<String, Arc<AtomicU8>>>;
fn add_reviewed_scope(plan: &mut Plan, selection: &[Value], explicit: bool) -> Result<()> {
    if !selection.is_empty() {
        let profiles = selection;
        if profiles.is_empty() || profiles.len() > 3 {
            return Err("SELECT_ONE_TO_THREE_REVIEWED_PROFILES".into());
        }
        plan.nodes.retain(|n| {
            !matches!(
                n.adapter.as_str(),
                "codex.skill" | "plugin.bundle" | "mcp.tool"
            )
        });
        let mut previous = "context".to_string();
        let mut chosen = Vec::new();
        for (index, selection) in profiles.iter().enumerate() {
            let p = compatibility::load(text(selection, "id")?, text(selection, "digest")?)?;
            let id = format!("reviewed-{index}");
            plan.nodes.push(compatibility::node(&p, &id, &[&previous]));
            chosen.push(json!({"name":p.name,"kind":p.kind,"profileId":p.id,"reason":"Selected reviewed metadata and immutable permission scope; readiness is rechecked before every effect","selection":if explicit{"EXPLICIT"}else{"AUTOMATIC_REVIEWED_SOURCE_MATCH"}}));
            previous = id;
        }
        for node in &mut plan.nodes {
            if node.adapter == "builtin.report" {
                node.dependencies.push(previous.clone());
            }
        }
        plan.scope="Approved third-party instruction-only analysis and/or public read-only MCP, then hash-verified report. Semantic task verification remains independent.".into();
        plan.goal_verifiable = false;
        plan.routing["reviewedSelections"] = json!(chosen);
        plan.routing["selectionRule"] = json!(
            "Reviewed-source task matching and optional explicit selection; concrete scopes still require approval and fresh runtime discovery."
        );
        plan.routing["reviewedProjectInputs"] = compatibility::project_inputs(&plan.project_root)?;
        plan.routing["publicMcpArguments"] = compatibility::public_inputs(plan)?;
        plan.routing["externalReady"] = json!(false);
        plan.validate()?;
    }
    Ok(())
}
fn controls() -> &'static Controls {
    static C: OnceLock<Controls> = OnceLock::new();
    C.get_or_init(Default::default)
}
fn instance() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}
type Sink = Arc<dyn Fn(Value) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
pub fn set_event_sink(sink: Sink) {
    let _ = SINK.set(sink);
}
fn publish(event: Value) {
    if let Some(sink) = SINK.get() {
        sink(event)
    }
}
fn clean(s: &str) -> String {
    crate::intelligence::clean(s)
}
fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("MISSING_{}", key.to_uppercase()))
}
pub(crate) fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return windows_sys::Win32::Foundation::GetLastError() == 5;
            }
            let mut code = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok == 0 || code == 259
        }
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from(format!("/proc/{pid}")).exists()
    }
}
impl Database {
    pub(crate) async fn save_routing_draft(
        &self,
        decision: &str,
        goal: String,
        project_root: String,
        evidence: Value,
        reviewed: Vec<Value>,
        first_party: bool,
    ) -> Result<Value> {
        let _guard = MUTATION.lock().await;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT workflow_id FROM routing_handoffs WHERE decision_id=?")
                .bind(decision)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
        let mut previous = None;
        if let Some(workflow) = existing {
            let plan = self.load_plan(&workflow, None).await?;
            if !first_party
                || plan.routing["projectRouting"]["executionProfile"] == "FIRST_PARTY_CODEX_READ"
            {
                return Ok(
                    json!({"workflowId":workflow,"plan":plan,"digest":plan.digest(),"existing":true}),
                );
            }
            if plan.revision != 1 {
                return Err("ROUTING_DRAFT_ALREADY_EDITED".into());
            }
            previous = Some(plan);
        }
        let mut plan = planner::generate(goal, project_root, &[], None)?;
        plan.routing["projectRouting"] = evidence;
        plan.routing["externalReady"] = json!(false);
        plan.routing["selectionRule"] = json!(
            "Project routing supplies an evidence-backed proposal. Mission Control owns approval and execution."
        );
        if first_party {
            plan.routing["projectRouting"]["executionProfile"] = json!("FIRST_PARTY_CODEX_READ");
            plan.scope = "Selected project metadata, a sealed first-party Codex fixture read and a digest-verified local report. Recommended external capabilities remain advisory.".into();
            let mut probe = types::base_node(
                "codex-fixture-read",
                "codex.fixture_read",
                "Read the sealed fixture with trusted Codex",
                &["context"],
            );
            probe.timeout_ms = 30_000;
            plan.nodes
                .iter_mut()
                .find(|node| node.id == "report")
                .ok_or("ROUTING_REPORT_MISSING")?
                .dependencies
                .push(probe.id.clone());
            plan.nodes.push(probe);
        } else {
            add_reviewed_scope(&mut plan, &reviewed, true)?;
        }
        self.source_check(&plan).await?;
        if let Some(previous) = previous {
            plan.workflow_id = previous.workflow_id;
            plan.revision = previous.revision + 1;
            self.save_plan(&plan, false).await?;
        } else {
            self.save_plan_with_handoff(&plan, true, Some(decision))
                .await?;
        }
        Ok(
            json!({"workflowId":plan.workflow_id,"plan":plan,"digest":plan.digest(),"existing":false}),
        )
    }

    pub async fn initialize_orchestration(&self) -> Result<Value> {
        let _guard = MUTATION.lock().await;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        if (10..=12).contains(&version) {
            return Ok(json!({"schema":version}));
        }
        if version != 9 {
            return Err("SCHEMA_9_REQUIRED".into());
        }
        let root =
            std::path::Path::new("D:/PersonalProject_DevTools/SkillVerseOrchestrationBackups");
        std::fs::create_dir_all(root).map_err(|_| "BACKUP_DIRECTORY")?;
        let backup = root.join(format!("schema9-{}.sqlite3", uuid::Uuid::new_v4()));
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_string_lossy().as_ref())
            .execute(&self.pool)
            .await
            .map_err(|_| "BACKUP_FAILED")?;
        let restored = backup.with_extension("restored.sqlite3");
        std::fs::copy(&backup, &restored).map_err(|_| "RESTORE_FAILED")?;
        let baseline = crate::capability_identity::database_fingerprint(&self.pool).await?;
        for p in [&backup, &restored] {
            let copy = Self::open_read_only(p.clone()).await?;
            copy.integrity_check().await?;
            if crate::capability_identity::database_fingerprint(&copy.pool).await? != baseline {
                return Err("BACKUP_RESTORE_MISMATCH".into());
            }
            copy.pool.close().await;
        }
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        for stmt in include_str!("../../migrations/0010_orchestration.sql")
            .split(';')
            .filter(|s| !s.trim().is_empty())
        {
            sqlx::query(stmt)
                .execute(&mut *tx)
                .await
                .map_err(|_| "ORCHESTRATION_MIGRATION_ROLLED_BACK")?;
        }
        tx.commit().await.map_err(|_| "MIGRATION_COMMIT")?;
        self.integrity_check().await?;
        Ok(json!({"schema":10,"backup":backup,"restored":restored,"verified":true}))
    }
    async fn event(
        &self,
        w: &str,
        r: Option<&str>,
        n: Option<&str>,
        event: &str,
        mut data: Value,
    ) -> Result<()> {
        if let Some(fields) = data.as_object_mut() {
            fields.insert("appSessionId".into(), json!(instance()));
        }
        let result=sqlx::query("INSERT INTO execution_events(workflow_id,execution_id,node_id,event,data_json,created_at) VALUES(?,?,?,?,?,?)").bind(w).bind(r).bind(n).bind(event).bind(data.to_string()).bind(now()).execute(&self.pool).await.map_err(|_|"EVENT_STORE_UNAVAILABLE")?;
        publish(
            json!({"workflowId":w,"executionId":r,"nodeId":n,"event":event,"sequence":result.last_insert_rowid()}),
        );
        Ok(())
    }
    pub async fn recover_orchestration(&self) -> Result<Value> {
        let has: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='execution_runs')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|_| "DATABASE")?;
        if !has {
            return Ok(json!({"recovered":0}));
        }
        let rows=sqlx::query("SELECT id,workflow_id,owner_pid FROM execution_runs WHERE status IN('RUNNING','QUEUED','VALIDATING')").fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
        let mut recovered = 0;
        for row in rows {
            let pid = row.get::<Option<i64>, _>("owner_pid").unwrap_or(0) as u32;
            if pid > 0 && pid_alive(pid) {
                continue;
            }
            let id: String = row.get("id");
            let w: String = row.get("workflow_id");
            let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
            sqlx::query("UPDATE execution_steps SET status='INTERRUPTED',error_type='OWNER_PROCESS_EXITED',finished_at=? WHERE execution_id=? AND status='RUNNING'").bind(now()).bind(&id).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            sqlx::query("UPDATE execution_runs SET status='INTERRUPTED',last_error='OWNER_PROCESS_EXITED_EXPLICIT_RESUME_REQUIRED',owner_pid=NULL,owner_token=NULL WHERE id=?").bind(&id).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            tx.commit().await.map_err(|_| "DATABASE")?;
            self.event(
                &w,
                Some(&id),
                None,
                "INTERRUPTED",
                json!({"automaticReplay":false}),
            )
            .await?;
            recovered += 1;
        }
        Ok(json!({"recovered":recovered,"automaticReplay":false}))
    }
    async fn load_plan(&self, id: &str, revision: Option<i64>) -> Result<Plan> {
        let raw:Option<String>=if let Some(v)=revision{sqlx::query_scalar("SELECT plan_json FROM workflow_versions WHERE workflow_id=? AND revision=?").bind(id).bind(v).fetch_optional(&self.pool).await}else{sqlx::query_scalar("SELECT plan_json FROM workflow_versions WHERE workflow_id=? ORDER BY revision DESC LIMIT 1").bind(id).fetch_optional(&self.pool).await}.map_err(|_|"DATABASE")?;
        serde_json::from_str(&raw.ok_or("WORKFLOW_NOT_FOUND")?)
            .map_err(|_| "INVALID_STORED_PLAN".into())
    }
    async fn save_plan(&self, plan: &Plan, new: bool) -> Result<()> {
        self.save_plan_with_handoff(plan, new, None).await
    }
    async fn save_plan_with_handoff(
        &self,
        plan: &Plan,
        new: bool,
        decision: Option<&str>,
    ) -> Result<()> {
        plan.validate()?;
        if !new {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_runs WHERE workflow_id=? AND status IN('RUNNING','QUEUED'))").bind(&plan.workflow_id).fetch_one(&self.pool).await.map_err(|_|"DATABASE")?;
            if active {
                return Err("PAUSE_OR_CANCEL_BEFORE_EDITING_PLAN".into());
            }
        }
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        if !new {
            sqlx::query("UPDATE execution_runs SET status='BLOCKED',last_error='PLAN_SUPERSEDED_APPROVAL_INVALIDATED' WHERE workflow_id=? AND status='WAITING_APPROVAL'").bind(&plan.workflow_id).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
        }

        if new {
            sqlx::query("INSERT INTO workflows VALUES(?,?,?,?,?)")
                .bind(&plan.workflow_id)
                .bind(&plan.goal)
                .bind(json!({"projectRoot":plan.project_root}).to_string())
                .bind(now())
                .bind(now())
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
        } else {
            sqlx::query("UPDATE workflows SET title=?,context_json=?,updated_at=? WHERE id=?")
                .bind(&plan.goal)
                .bind(json!({"projectRoot":plan.project_root}).to_string())
                .bind(now())
                .bind(&plan.workflow_id)
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
        }
        sqlx::query("INSERT INTO workflow_versions VALUES(?,?,?,?,?)")
            .bind(&plan.workflow_id)
            .bind(plan.revision)
            .bind(serde_json::to_string(plan).unwrap())
            .bind(plan.digest())
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|_| "VERSION_CONFLICT")?;
        for n in &plan.nodes {
            sqlx::query("INSERT INTO workflow_nodes VALUES(?,?,?,?)")
                .bind(&plan.workflow_id)
                .bind(plan.revision)
                .bind(&n.id)
                .bind(serde_json::to_string(n).unwrap())
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
        }
        for n in &plan.nodes {
            for dep in &n.dependencies {
                sqlx::query("INSERT INTO workflow_edges VALUES(?,?,?,?)")
                    .bind(&plan.workflow_id)
                    .bind(plan.revision)
                    .bind(dep)
                    .bind(&n.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| "DATABASE")?;
            }
        }
        if let Some(decision) = decision {
            sqlx::query("INSERT INTO routing_handoffs VALUES(?,?,?)")
                .bind(decision)
                .bind(&plan.workflow_id)
                .bind(now())
                .execute(&mut *tx)
                .await
                .map_err(|_| "HANDOFF_CONFLICT")?;
        }
        tx.commit().await.map_err(|_| "DATABASE")?;
        self.event(
            &plan.workflow_id,
            None,
            None,
            "PLAN_SAVED",
            json!({"revision":plan.revision,"digest":plan.digest()}),
        )
        .await
    }
    async fn source_check(&self, plan: &Plan) -> Result<()> {
        if plan.nodes.iter().any(|n| n.adapter == "mcp.reviewed")
            && compatibility::public_inputs(plan)? != plan.routing["publicMcpArguments"]
        {
            return Err("MCP_ARGUMENTS_CHANGED_REPLAN_REQUIRED".into());
        }
        if plan
            .nodes
            .iter()
            .any(|n| matches!(n.adapter.as_str(), "skill.reviewed" | "plugin.reviewed"))
            && compatibility::project_inputs(&plan.project_root)?
                != plan.routing["reviewedProjectInputs"]
        {
            return Err("PROJECT_INPUT_CHANGED_REPLAN_REQUIRED".into());
        }
        let subjects = self.intelligence_subjects().await?;
        for n in &plan.nodes {
            if n.adapter.ends_with(".reviewed") {
                let profile = compatibility::validate(n)?;
                compatibility::check(&profile)?;
            }
            if let Some(id) = &n.capability_id {
                let s = subjects
                    .iter()
                    .find(|s| &s.logical_id == id)
                    .ok_or("CAPABILITY_NO_LONGER_VISIBLE")?;
                if n.capability_type != s.kind
                    || n.capability_version.as_deref()
                        != Some(crate::learning::source_version(s).as_str())
                {
                    return Err("CAPABILITY_VERSION_CHANGED_REPLAN_REQUIRED".into());
                }
            }
        }
        Ok(())
    }
    fn scope(plan: &Plan, run: &str, workspace: &str) -> Value {
        let inference = plan.nodes.iter().any(|n| {
            n.adapter == "codex.skill_fixture"
                || matches!(n.adapter.as_str(), "skill.reviewed" | "plugin.reviewed")
        });
        let loopback = plan.nodes.iter().any(|n| n.adapter == "mcp.fixture");
        json!({"workflowId":plan.workflow_id,"revision":plan.revision,"planDigest":plan.digest(),"executionId":run,"projectRoot":plan.project_root,"reviewedProjectInputs":plan.routing["reviewedProjectInputs"],"publicMcpArguments":plan.routing["publicMcpArguments"],"readFileNames":["package.json","README.md","tsconfig.json","sample.tsx","sample.js","input.json"],"reviewedCapabilities":plan.nodes.iter().filter(|n|n.adapter.ends_with(".reviewed")).map(compatibility::scope).collect::<Vec<_>>(),"writeRoot":workspace,"network":([if inference{Some("CODEX_SERVICE_ONLY")}else{None},if loopback{Some("OWNED_LOOPBACK_FIXTURE_ONLY")}else{None},if plan.nodes.iter().any(|n|n.adapter=="mcp.reviewed"){Some("EXACT_PUBLIC_ENDPOINT_AND_TOOL_IN_REVIEWED_CAPABILITIES")}else{None}].into_iter().flatten().collect::<Vec<_>>()),"modelCalls":inference,"maxCodexTurns":plan.nodes.iter().filter(|n|matches!(n.adapter.as_str(),"codex.skill_fixture"|"skill.reviewed"|"plugin.reviewed")).count(),"account":if inference{"EXISTING_CHATGPT_LOGIN_ONLY_NO_NEW_LOGIN_NO_API_KEY"}else{"NONE"},"unknownScripts":false,"globalCodexWrites":false,"adapters":plan.nodes.iter().map(|n|json!({"nodeId":n.id,"adapter":n.adapter,"risk":n.risk})).collect::<Vec<_>>()})
    }
    async fn state_workflow(&self, id: Option<&str>) -> Result<Value> {
        let rows=sqlx::query("SELECT w.id,w.title,w.updated_at,MAX(v.revision) AS revision FROM workflows w JOIN workflow_versions v ON v.workflow_id=w.id GROUP BY w.id ORDER BY w.updated_at DESC LIMIT 100").fetch_all(&self.pool).await.map_err(|_|"DATABASE")?;
        let workflows:Vec<_>=rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"title":r.get::<String,_>("title"),"revision":r.get::<i64,_>("revision"),"updatedAt":r.get::<i64,_>("updated_at")})).collect();
        let selected = id.or_else(|| workflows.first().and_then(|w| w["id"].as_str()));
        let mut result = json!({"ready":true,"workflows":workflows,"liveModel":"NOT VERIFIED","defaultProject":std::env::current_dir().ok().map(|p|p.to_string_lossy().into_owned())});
        if let Some(w) = selected {
            // Fetch a consistent page in bulk so polling does not query each run separately.
            let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
            let raw: String = sqlx::query_scalar(
                "SELECT plan_json FROM workflow_versions WHERE workflow_id=? ORDER BY revision DESC LIMIT 1",
            ).bind(w).fetch_optional(&mut *tx).await.map_err(|_| "DATABASE")?.ok_or("WORKFLOW_NOT_FOUND")?;
            let plan: Plan = serde_json::from_str(&raw).map_err(|_| "PLAN_INVALID")?;
            let rows = sqlx::query(
                "SELECT r.*,v.plan_json FROM execution_runs r JOIN workflow_versions v
                 ON v.workflow_id=r.workflow_id AND v.revision=r.revision
                 WHERE r.workflow_id=? ORDER BY r.created_at DESC,r.id DESC LIMIT 50",
            )
            .bind(w)
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| "DATABASE")?;
            let mut history: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            let mut receipts: BTreeMap<String, Vec<sqlx::sqlite::SqliteRow>> = BTreeMap::new();
            let mut approvals: BTreeMap<String, String> = BTreeMap::new();
            if !rows.is_empty() {
                let newest = "SELECT id FROM execution_runs WHERE workflow_id=? ORDER BY created_at DESC,id DESC LIMIT 50";
                for row in sqlx::query(&format!(
                    "SELECT * FROM execution_steps WHERE execution_id IN ({newest}) ORDER BY started_at,attempt,id"
                )).bind(w).fetch_all(&mut *tx).await.map_err(|_| "DATABASE")? {
                    history.entry(row.get("execution_id")).or_default().push(step_json(&row));
                }
                for row in sqlx::query(&format!(
                    "SELECT * FROM execution_evidence WHERE execution_id IN ({newest}) ORDER BY created_at,id"
                )).bind(w).fetch_all(&mut *tx).await.map_err(|_| "DATABASE")? {
                    receipts.entry(row.get("execution_id")).or_default().push(row);
                }
                for (id, scope) in sqlx::query_as::<_, (String, String)>(&format!(
                    "SELECT execution_id,scope_json FROM execution_approvals WHERE decision='APPROVED'
                     AND execution_id IN ({newest}) ORDER BY created_at DESC,id DESC"
                )).bind(w).fetch_all(&mut *tx).await.map_err(|_| "APPROVAL_HISTORY_UNAVAILABLE")? {
                    approvals.entry(id).or_insert(scope);
                }
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            let mut runs = Vec::new();
            for row in rows {
                let id: String = row.get("id");
                let revision: i64 = row.get("revision");
                let run_plan: Plan = serde_json::from_str(row.get::<&str, _>("plan_json"))
                    .map_err(|_| "PLAN_INVALID")?;
                let workspace: String = row.get("workspace");
                let scope = match row.get::<&str, _>("status") {
                    "SUCCEEDED" | "CANCELLED" => approvals.remove(&id),
                    _ => None,
                }
                .map(|raw| serde_json::from_str(&raw).map_err(|_| "APPROVAL_HISTORY_INVALID"))
                .transpose()?
                .unwrap_or_else(|| Self::scope(&run_plan, &id, &workspace));
                let steps = history.remove(&id).unwrap_or_default();
                let evidence:Vec<_>=receipts.remove(&id).unwrap_or_default().iter().map(|r|{let path:String=r.get("evidence_reference");let hash:String=r.get("evidence_hash");let intact=security::within_owned_root(std::path::Path::new(&path))&&security::read(std::path::Path::new(&path),262144).is_ok_and(|b|digest(&b)==hash);json!({"id":r.get::<String,_>("id"),"nodeId":r.get::<String,_>("node_id"),"location":path,"sha256":hash,"intact":intact,"policy":r.get::<String,_>("verification_policy")})}).collect();
                let report_node = run_plan
                    .nodes
                    .iter()
                    .find(|n| n.adapter == "builtin.report")
                    .map(|n| n.id.as_str());
                let report_intact = report_node.is_none_or(|node| {
                    steps
                        .iter()
                        .rev()
                        .find(|s| s["nodeId"] == node && s["status"] == "SUCCEEDED")
                        .is_some_and(|s| {
                            s["output"]["sha256"].as_str().is_some_and(|hash| {
                                let path = std::path::PathBuf::from(&workspace).join("report.json");
                                security::within_owned_root(&path)
                                    && security::read(&path, 262144)
                                        .is_ok_and(|b| digest(&b) == hash)
                            })
                        })
                });
                let mode: String = row.get("mode");
                let intact = !evidence.is_empty()
                    && evidence.iter().all(|e| e["intact"] == true)
                    && report_intact;
                let verification = if mode == "DRY_RUN" {
                    "NOT_TESTED"
                } else if row.get::<String, _>("status") == "SUCCEEDED" && intact {
                    "PASSED_WITHIN_ADAPTER_SCOPE"
                } else {
                    "INCONCLUSIVE"
                };
                runs.push(json!({"id":id,"revision":revision,"plan":run_plan,"mode":row.get::<String,_>("mode"),"status":row.get::<String,_>("status"),"lastError":row.get::<String,_>("last_error"),"createdAt":row.get::<i64,_>("created_at"),"startedAt":row.get::<Option<i64>,_>("started_at"),"finishedAt":row.get::<Option<i64>,_>("finished_at"),"workspace":workspace,"verificationStatus":verification,"reportIntact":report_intact,"goalVerified":row.get::<String,_>("status")=="SUCCEEDED"&&run_plan.goal_verifiable&&intact,"scopeDigest":digest(scope.to_string().as_bytes()),"scope":scope,"steps":steps,"evidence":evidence}));
            }
            result["plan"] = serde_json::to_value(&plan).unwrap();
            result["digest"] = json!(plan.digest());
            result["runs"] = json!(runs);
        }
        Ok(result)
    }
    async fn run_steps(&self, id: &str) -> Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT * FROM execution_steps WHERE execution_id=? ORDER BY started_at,attempt",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "DATABASE")?;
        Ok(rows.iter().map(step_json).collect())
    }
    fn launch(&self, run: String) {
        let control = Arc::new(AtomicU8::new(0));
        if let Ok(mut map) = controls().lock() {
            if map.contains_key(&run) {
                return;
            }
            map.insert(run.clone(), control.clone());
        }
        let db = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = db.drive(&run, control.clone()).await {
                control.store(3, Ordering::SeqCst);
                let _=sqlx::query("UPDATE execution_runs SET status='BLOCKED',last_error=? WHERE id=? AND status IN('RUNNING','QUEUED')").bind(clean(&error)).bind(&run).execute(&db.pool).await;
            }
            if let Ok(mut map) = controls().lock() {
                map.remove(&run);
            }
        });
    }
    pub async fn orchestration(&self, action: &str, args: Value) -> Result<Value> {
        if action == "initialize" {
            return self.initialize_orchestration().await;
        }
        let ready: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='workflows')")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
        if !ready {
            return if action == "state" {
                Ok(json!({"ready":false}))
            } else {
                Err("ENABLE_VERIFIED_ORCHESTRATION_STORAGE".into())
            };
        }
        if action == "state" {
            return self.state_workflow(args["workflowId"].as_str()).await;
        }
        if action == "export-diagnostics" {
            return self
                .export_execution_diagnostics(text(&args, "executionId")?)
                .await;
        }
        if action == "events" {
            let w = text(&args, "workflowId")?;
            let rows=if args["latest"]==true{sqlx::query("SELECT * FROM (SELECT * FROM execution_events WHERE workflow_id=? ORDER BY sequence DESC LIMIT 200) ORDER BY sequence").bind(w).fetch_all(&self.pool).await}else{sqlx::query("SELECT * FROM execution_events WHERE workflow_id=? AND sequence>? ORDER BY sequence LIMIT 200").bind(w).bind(args["after"].as_i64().unwrap_or(0).max(0)).fetch_all(&self.pool).await}.map_err(|_|"DATABASE")?;
            return Ok(
                json!({"events":rows.iter().map(|r|json!({"sequence":r.get::<i64,_>("sequence"),"nodeId":r.get::<Option<String>,_>("node_id"),"executionId":r.get::<Option<String>,_>("execution_id"),"event":r.get::<String,_>("event"),"at":r.get::<i64,_>("created_at"),"data":serde_json::from_str::<Value>(&r.get::<String,_>("data_json")).unwrap_or(Value::Null)})).collect::<Vec<_>>()}),
            );
        }
        if action == "read-evidence" {
            let run = text(&args, "executionId")?;
            let node = text(&args, "nodeId")?;
            let record:Option<(String,String)>=sqlx::query_as("SELECT evidence_reference,evidence_hash FROM execution_evidence WHERE execution_id=? AND node_id=?").bind(run).bind(node).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?;
            let (path, hash) = record.ok_or("EVIDENCE_NOT_FOUND")?;
            let path = std::path::PathBuf::from(path);
            if !security::within_owned_root(&path) {
                return Err("EVIDENCE_SCOPE".into());
            }
            let bytes = security::read(&path, 262144)?;
            if digest(&bytes) != hash {
                return Err("EVIDENCE_HASH_MISMATCH".into());
            }
            return Ok(
                json!({"sha256":hash,"data":serde_json::from_slice::<Value>(&bytes).map_err(|_|"INVALID_EVIDENCE_JSON")?}),
            );
        }
        if action == "compatibility-list" {
            return compatibility::list();
        }
        if action == "audit-capability-environment" {
            let project = security::project(text(&args, "projectRoot")?)?;
            return tauri::async_runtime::spawn_blocking(move||{
                let user=std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).ok_or("USER_HOME_REQUIRED")?;
                let codex=std::env::var_os("CODEX_HOME").map(std::path::PathBuf::from).unwrap_or_else(||user.join(".codex"));
                let report=crate::capability_identity::scan(&crate::capability_identity::ScanContext::selected(user,codex,Some(project)))?;
                Ok(json!({"scope":"CURRENT_READ_ONLY_DISCOVERY_NO_DATABASE_RECONCILIATION","skills":report.skills,"plugins":report.plugins,"mcpServers":report.mcp_servers,"warnings":report.warnings,"currentObservations":report.identity.as_ref().map(|x|x.observations.len()),"states":{"CODEX_RECOGNIZED":"NOT VERIFIED","CONNECTED":"NOT VERIFIED","CALLABLE":"NOT VERIFIED","EXECUTED":"NOT VERIFIED","VERIFIED":"NOT VERIFIED"},"unknownProcessesStarted":false,"modelCalls":0}))
            }).await.map_err(|_|"CAPABILITY_AUDIT_TASK")?;
        }
        if action == "probe-reviewed-plugin" {
            if args["confirmed"] != true {
                return Err("PLUGIN_PROBE_EXACT_SOURCE_CONFIRMATION_REQUIRED".into());
            }
            let id = text(&args, "id")?.to_string();
            let digest = text(&args, "digest")?.to_string();
            return tauri::async_runtime::spawn_blocking(move || {
                compatibility::probe_plugin(&id, &digest)
            })
            .await
            .map_err(|_| "PLUGIN_PROBE_TASK")?;
        }
        if action == "review-capability" {
            return compatibility::review(&args).await;
        }
        if action == "catalog" {
            return Ok(
                json!({"subjects":self.intelligence_subjects().await?.into_iter().map(|s|json!({"logicalId":s.logical_id,"entityId":s.entity_id,"name":s.name,"kind":s.kind,"version":crate::learning::source_version(&s),"runtime":"NOT VERIFIED"})).collect::<Vec<_>>()}),
            );
        }
        if action == "runtime-probe" {
            let proof = tauri::async_runtime::spawn_blocking(runtime::probe)
                .await
                .map_err(|_| "RUNTIME_PROBE_TASK")??;
            sqlx::query("INSERT INTO runtime_observations VALUES(?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind("CODEX_STDIO_V2")
                .bind(proof.to_string())
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            return Ok(proof);
        }
        if action == "cancel" || action == "pause" {
            let run = text(&args, "executionId")?;
            let token = controls().lock().ok().and_then(|m| m.get(run).cloned());
            if let Some(token) = token {
                if action == "cancel" {
                    adapters::resolve("builtin.wait")?.cancel(&token)
                } else {
                    token.store(1, Ordering::SeqCst);
                }
                let workflow: Option<String> =
                    sqlx::query_scalar("SELECT workflow_id FROM execution_runs WHERE id=?")
                        .bind(run)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(|_| "DATABASE")?;
                if let Some(w) = workflow {
                    self.event(
                        &w,
                        Some(run),
                        None,
                        if action == "cancel" {
                            "CANCEL_REQUESTED"
                        } else {
                            "PAUSE_REQUESTED"
                        },
                        json!({"waitForWorkerAcknowledgement":true}),
                    )
                    .await?;
                    return Ok(json!({"requested":true}));
                }
            }
        }
        let _guard = MUTATION.lock().await;
        if action == "plan" {
            let goal = clean(text(&args, "goal")?);
            let project = security::project(text(&args, "projectRoot")?)?;
            let subjects = self.intelligence_subjects().await?;
            let mut plan = planner::generate(
                goal,
                project.to_string_lossy().into_owned(),
                &subjects,
                args["preferredCapability"].as_str(),
            )?;
            let suggested = compatibility::suggestions(&plan.goal);
            let selection = args["reviewedProfiles"]
                .as_array()
                .cloned()
                .unwrap_or(suggested);
            add_reviewed_scope(&mut plan, &selection, args["reviewedProfiles"].is_array())?;
            self.save_plan(&plan, true).await?;
            return Ok(json!({"workflowId":plan.workflow_id,"plan":plan,"digest":plan.digest()}));
        }
        if action == "save-plan" {
            let mut plan: Plan =
                serde_json::from_value(args["plan"].clone()).map_err(|_| "INVALID_PLAN_FIELDS")?;
            let old = self.load_plan(&plan.workflow_id, None).await?;
            if args["expectedDigest"] != old.digest() {
                return Err("PLAN_CHANGED_RELOAD".into());
            }
            plan.revision = old.revision + 1;
            plan.goal = clean(&plan.goal);
            plan.project_root = security::project(&plan.project_root)?
                .to_string_lossy()
                .into_owned();
            plan.routing = old.routing;
            if plan.routing["projectRouting"]["executionProfile"] == "FIRST_PARTY_CODEX_READ"
                && (plan.goal != old.goal || plan.project_root != old.project_root)
            {
                return Err("ROUTING_GOAL_OR_PROJECT_CHANGED_REANALYZE".into());
            }
            plan.goal_verifiable = false; // Edited plans need a new task-specific success policy.
            for n in &mut plan.nodes {
                n.description = clean(&n.description);
            }
            self.source_check(&plan).await?;
            self.save_plan(&plan, false).await?;
            return Ok(json!({"plan":plan,"digest":plan.digest()}));
        }
        if action == "validate" {
            let plan = self.load_plan(text(&args, "workflowId")?, None).await?;
            let order = plan.validate()?;
            self.source_check(&plan).await?;
            return Ok(
                json!({"valid":true,"order":order,"adapters":plan.nodes.iter().map(|n|adapters::resolve(&n.adapter).and_then(|a|a.prepare(n)).unwrap_or_else(|e|json!({"ready":false,"reason":e}))).collect::<Vec<_>>(),"executionOccurred":false}),
            );
        }
        if action == "start" {
            let w = text(&args, "workflowId")?;
            let plan = self.load_plan(w, None).await?;
            if args["digest"] != plan.digest() {
                return Err("PLAN_CHANGED_REVIEW_REQUIRED".into());
            }
            plan.validate()?;
            if let Err(error) = self.source_check(&plan).await {
                let blocked_node = plan
                    .nodes
                    .iter()
                    .find(|n| {
                        n.adapter.ends_with(".reviewed")
                            && compatibility::validate(n)
                                .and_then(|p| compatibility::check(&p))
                                .is_err()
                    })
                    .map(|n| n.id.as_str());
                self.event(
                    &plan.workflow_id,
                    None,
                    blocked_node,
                    "PRECHECK_BLOCKED",
                    json!({"error":clean(&error),"executionOccurred":false}),
                )
                .await?;
                return Err(error);
            }
            security::project(&plan.project_root)?;
            let mode = text(&args, "mode")?;
            if !["DRY_RUN", "SANDBOX_EXECUTION", "APPROVED_EXECUTION"].contains(&mode) {
                return Err("PLAN_ONLY_HAS_NO_EXECUTION_OR_UNKNOWN_MODE".into());
            }
            let key = text(&args, "requestKey")?;
            if key.len() > 100 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err("INVALID_IDEMPOTENCY_KEY".into());
            }
            let existing:Option<String>=sqlx::query_scalar("SELECT id FROM execution_runs WHERE workflow_id=? AND revision=? AND request_key=?").bind(w).bind(plan.revision).bind(key).fetch_optional(&self.pool).await.map_err(|_|"DATABASE")?;
            if let Some(id) = existing {
                return Ok(json!({"executionId":id,"duplicatePrevented":true}));
            }
            let active:i64=sqlx::query_scalar("SELECT count(*) FROM execution_runs WHERE workflow_id=? AND status IN('QUEUED','RUNNING','WAITING_APPROVAL')").bind(w).fetch_one(&self.pool).await.map_err(|_|"DATABASE")?;
            if active > 0 {
                return Err("WORKFLOW_ALREADY_ACTIVE".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let workspace = security::root()
                .join("workflows")
                .join(w)
                .join(&id)
                .to_string_lossy()
                .into_owned();
            let state = if mode == "DRY_RUN" {
                "QUEUED"
            } else {
                "WAITING_APPROVAL"
            };
            sqlx::query("INSERT INTO execution_runs(id,workflow_id,revision,mode,status,request_key,plan_digest,workspace,owner_pid,owner_token,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(&id).bind(w).bind(plan.revision).bind(mode).bind(state).bind(key).bind(plan.digest()).bind(&workspace).bind(std::process::id() as i64).bind(instance()).bind(now()).execute(&self.pool).await.map_err(|_|"EXECUTION_CONFLICT")?;
            self.event(
                w,
                Some(&id),
                None,
                state,
                json!({"mode":mode,"executionOccurred":false}),
            )
            .await?;
            let scope = Self::scope(&plan, &id, &workspace);
            if mode == "DRY_RUN" {
                self.launch(id.clone())
            }
            return Ok(
                json!({"executionId":id,"status":state,"scopeDigest":digest(scope.to_string().as_bytes()),"scope":scope}),
            );
        }
        let run = text(&args, "executionId")?;
        let row = sqlx::query("SELECT * FROM execution_runs WHERE id=?")
            .bind(run)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| "DATABASE")?
            .ok_or("EXECUTION_NOT_FOUND")?;
        let w: String = row.get("workflow_id");
        let status: String = row.get("status");
        let plan = self.load_plan(&w, Some(row.get("revision"))).await?;
        if action == "cancel" || action == "pause" {
            let token = controls().lock().ok().and_then(|m| m.get(run).cloned());
            if let Some(token) = token {
                if action == "cancel" {
                    adapters::resolve("builtin.wait")?.cancel(&token)
                } else {
                    token.store(1, Ordering::SeqCst);
                }
                self.event(
                    &w,
                    Some(run),
                    None,
                    if action == "cancel" {
                        "CANCEL_REQUESTED"
                    } else {
                        "PAUSE_REQUESTED"
                    },
                    json!({"waitForWorkerAcknowledgement":true}),
                )
                .await?;
                return Ok(json!({"requested":true}));
            }
            if !["WAITING_APPROVAL", "INTERRUPTED", "RECOVERABLE", "BLOCKED"]
                .contains(&status.as_str())
            {
                return Err("EXECUTION_NOT_ACTIVE".into());
            }
            sqlx::query("UPDATE execution_runs SET status=?,finished_at=? WHERE id=?")
                .bind(if action == "cancel" {
                    "CANCELLED"
                } else {
                    "RECOVERABLE"
                })
                .bind(now())
                .bind(run)
                .execute(&self.pool)
                .await
                .map_err(|_| "DATABASE")?;
            return Ok(json!({"saved":true}));
        }
        if action == "approve" || action == "deny" || action == "resume" {
            if (action == "approve" || action == "deny") && status != "WAITING_APPROVAL" {
                return Err("APPROVAL_STATE_MISMATCH".into());
            }
            if action == "resume"
                && !matches!(
                    status.as_str(),
                    "INTERRUPTED" | "RECOVERABLE" | "FAILED" | "BLOCKED"
                )
            {
                return Err("NOT_RECOVERABLE".into());
            }
            if action != "deny" {
                self.source_check(&plan).await?;
                if args["confirmed"] != true {
                    return Err("EXPLICIT_SCOPE_CONFIRMATION_REQUIRED".into());
                }
            }
            let workspace: String = row.get("workspace");
            let scope = Self::scope(&plan, run, &workspace);
            let scope_hash = digest(scope.to_string().as_bytes());
            if args["scopeDigest"] != scope_hash {
                return Err("APPROVAL_SCOPE_CHANGED".into());
            }
            let latest = self.load_plan(&w, None).await?;
            if action != "deny" && latest.digest() != plan.digest() {
                return Err("PLAN_CHANGED_REPLAN_AND_NEW_APPROVAL".into());
            }
            let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
            sqlx::query("INSERT INTO execution_approvals VALUES(?,?,?,?,?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(run)
                .bind(plan.digest())
                .bind(&scope_hash)
                .bind(scope.to_string())
                .bind(if action == "deny" {
                    "DENIED"
                } else {
                    "APPROVED"
                })
                .bind(now() + 1800000)
                .bind(now())
                .execute(&mut *tx)
                .await
                .map_err(|_| "DATABASE")?;
            if action == "resume" {
                let uncertain:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_steps WHERE execution_id=? AND node_id IN (SELECT node_id FROM workflow_nodes WHERE workflow_id=? AND revision=? AND json_extract(node_json,'$.adapter') IN ('codex.skill_fixture','skill.reviewed','plugin.reviewed')) AND status IN('RUNNING','INTERRUPTED','FAILED','CANCELLED'))").bind(run).bind(&plan.workflow_id).bind(plan.revision).fetch_one(&mut *tx).await.map_err(|_|"DATABASE")?;
                if uncertain {
                    return Err(
                        "INFERENCE_MAY_HAVE_OCCURRED_NEW_EXPLICIT_EXECUTION_REQUIRED".into(),
                    );
                }

                if controls().lock().ok().is_some_and(|m| m.contains_key(run)) {
                    return Err("WORKER_STILL_STOPPING".into());
                }
                let owner = row.get::<Option<i64>, _>("owner_pid").unwrap_or(0) as u32;
                if owner != 0 && owner != std::process::id() && pid_alive(owner) {
                    return Err("ANOTHER_PROCESS_OWNS_THIS_EXECUTION".into());
                }
                sqlx::query("UPDATE execution_steps SET status='INTERRUPTED',error_type='EXPLICIT_RECOVERY',finished_at=? WHERE execution_id=? AND status='RUNNING'").bind(now()).bind(run).execute(&mut *tx).await.map_err(|_|"DATABASE")?;
            }
            let changed=sqlx::query("UPDATE execution_runs SET status=?,owner_pid=?,owner_token=?,last_error='',finished_at=NULL WHERE id=? AND status=?").bind(if action=="deny"{"CANCELLED"}else{"QUEUED"}).bind(std::process::id() as i64).bind(instance()).bind(run).bind(&status).execute(&mut *tx).await.map_err(|_|"EXECUTION_ALREADY_ACTIVE")?;
            if changed.rows_affected() != 1 {
                return Err("EXECUTION_STATE_CHANGED_RELOAD".into());
            }
            tx.commit().await.map_err(|_| "DATABASE")?;
            self.event(
                &w,
                Some(run),
                None,
                if action == "deny" {
                    "APPROVAL_DENIED"
                } else {
                    "SCOPE_APPROVED"
                },
                json!({"scopeDigest":scope_hash,"automaticReplay":false}),
            )
            .await?;
            if action != "deny" {
                self.launch(run.into())
            }
            return Ok(json!({"saved":true}));
        }
        Err("UNKNOWN_ORCHESTRATION_ACTION".into())
    }
}
