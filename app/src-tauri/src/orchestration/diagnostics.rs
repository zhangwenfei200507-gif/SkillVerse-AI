//! Export a bounded projection of the execution journal. Free-form event data,
//! scope, inputs, outputs, credentials and paths are outside this projection.
use super::*;

const MAX_EVENTS: i64 = 200;
const MAX_EXPORTS: usize = 32;
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn category(code: &str) -> (&'static str, &'static str) {
    match code {
        "RUNNING" | "NODE_RUNNING" | "NODE_TOOL_RUNNING" => ("INFO", "RUNNING"),
        "NODE_CONNECTING" => ("INFO", "CONNECTING"),
        "NODE_CALLABLE" => ("INFO", "CALLABLE"),
        "SUCCEEDED" => ("INFO", "SUCCEEDED"),
        "WAITING_APPROVAL" => ("INFO", "WAITING_APPROVAL"),
        "APPROVED" => ("INFO", "APPROVED"),
        "DENIED" | "APPROVAL_DENIED" => ("WARN", "APPROVAL_DENIED"),
        "CANCELLED" => ("WARN", "CANCELLED"),
        "PAUSED" | "RECOVERABLE" => ("WARN", "RECOVERABLE"),
        "INTERRUPTED" => ("WARN", "INTERRUPTED"),
        "BLOCKED" | "PRECHECK_BLOCKED" => ("WARN", "BLOCKED"),
        "FAILED" => ("ERROR", "FAILED"),
        "SKIPPED" => ("INFO", "SKIPPED"),
        _ => ("INFO", "OTHER"),
    }
}

fn opaque_ref(value: Option<&str>) -> Option<String> {
    value.map(|text| digest(text.as_bytes()))
}

fn uuid_string(value: Option<&str>) -> Option<String> {
    uuid::Uuid::parse_str(value?).ok().map(|id| id.to_string())
}

impl Database {
    pub(super) async fn export_execution_diagnostics(&self, input: &str) -> Result<Value> {
        let run = uuid_string(Some(input)).ok_or("DIAGNOSTICS_EXECUTION_ID")?;
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        let row = sqlx::query("SELECT workflow_id,revision,status,created_at,started_at,finished_at FROM execution_runs WHERE id=?")
            .bind(&run).fetch_optional(&mut *tx).await.map_err(|_| "DATABASE")?.ok_or("EXECUTION_NOT_FOUND")?;
        let workflow = uuid_string(Some(row.get::<&str, _>("workflow_id")))
            .ok_or("DIAGNOSTICS_WORKFLOW_ID")?;
        let journal = sqlx::query(
            "SELECT sequence,node_id,event,created_at,json_extract(data_json,'$.appSessionId') AS session
             FROM execution_events WHERE execution_id=? ORDER BY sequence DESC LIMIT ?",
        ).bind(&run).bind(MAX_EVENTS+1).fetch_all(&mut *tx).await.map_err(|_| "DATABASE")?;
        let step_rows = sqlx::query(
            "SELECT s.node_id,s.attempt,s.status,s.started_at,s.finished_at,
                    json_extract(n.node_json,'$.adapter') AS adapter,
                    json_extract(n.node_json,'$.capabilityId') AS capability,
                    json_extract(s.output_json,'$.activationEvidence.threadId') AS runtime_session,
                    json_extract(s.output_json,'$.activationEvidence.turnId') AS runtime_turn,
                    e.evidence_hash
             FROM execution_steps s
             JOIN execution_runs r ON r.id=s.execution_id
             LEFT JOIN workflow_nodes n ON n.workflow_id=r.workflow_id AND n.revision=r.revision AND n.node_id=s.node_id
             LEFT JOIN execution_evidence e ON e.execution_id=s.execution_id AND e.node_id=s.node_id
             WHERE s.execution_id=? ORDER BY s.started_at DESC,s.id DESC LIMIT 200",
        ).bind(&run).fetch_all(&mut *tx).await.map_err(|_| "DATABASE")?;
        let decisions = sqlx::query("SELECT decision,created_at FROM execution_approvals WHERE execution_id=? ORDER BY created_at DESC LIMIT 20")
            .bind(&run).fetch_all(&mut *tx).await.map_err(|_| "DATABASE")?;
        tx.commit().await.map_err(|_| "DATABASE")?;
        let events: Vec<_> = journal.iter().take(MAX_EVENTS as usize).rev().map(|event| {
            let (level, name) = category(event.get("event"));
            json!({"sequence":event.get::<i64,_>("sequence"),"nodeRef":opaque_ref(event.get("node_id")),
                "level":level,"event":name,"at":event.get::<i64,_>("created_at"),
                "appSessionId":uuid_string(event.get("session"))})
        }).collect();
        let steps: Vec<_> = step_rows.iter().rev().map(|step| {
            let adapter: Option<&str> = step.get("adapter");
            let hash = step.get::<Option<&str>, _>("evidence_hash").filter(|hash| hash.len()==64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
            json!({"nodeRef":opaque_ref(step.get("node_id")),"attempt":step.get::<i64,_>("attempt"),
                "adapter":adapter.filter(|name| adapters::resolve(name).is_ok()),
                "capabilityRef":opaque_ref(step.get("capability")),"status":category(step.get("status")).1,
                "startedAt":step.get::<i64,_>("started_at"),"finishedAt":step.get::<Option<i64>,_>("finished_at"),
                "runtimeSessionRef":opaque_ref(step.get("runtime_session")),"runtimeTurnRef":opaque_ref(step.get("runtime_turn")),
                "evidenceSha256":hash})
        }).collect();
        let approvals: Vec<_> = decisions.iter().rev().map(|decision| json!({
            "decision":category(decision.get("decision")).1, "at":decision.get::<i64,_>("created_at")
        })).collect();
        let envelope = json!({"format":"skillverse-diagnostics-v1","exportedAt":now(),
            "exportSessionId":instance(),"executionId":run,"workflowId":workflow,"revision":row.get::<i64,_>("revision"),
            "status":category(row.get("status")).1,"createdAt":row.get::<i64,_>("created_at"),
            "startedAt":row.get::<Option<i64>,_>("started_at"),"finishedAt":row.get::<Option<i64>,_>("finished_at"),
            "eventsTruncated":journal.len()>MAX_EVENTS as usize,"events":events,"steps":steps,"approvals":approvals});
        let bytes = serde_json::to_vec_pretty(&envelope).map_err(|_| "DIAGNOSTICS_ENCODE")?;
        if bytes.len() > 256 * 1024 {
            return Err("DIAGNOSTICS_SIZE_LIMIT".into());
        }
        let _lock = EXPORT_LOCK.lock().await;
        let folder = security::root().join("diagnostics");
        security::owned_dir(&folder)?;
        let count = std::fs::read_dir(&folder)
            .map_err(|_| "DIAGNOSTICS_DIRECTORY")?
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|file| file.file_name().to_string_lossy().starts_with("execution-"))
            })
            .take(MAX_EXPORTS)
            .count();
        if count >= MAX_EXPORTS {
            return Err("DIAGNOSTICS_EXPORT_LIMIT".into());
        }
        let path = folder.join(format!("execution-{run}-{}.json", uuid::Uuid::new_v4()));
        let hash = security::seal(&path, &bytes)?;
        Ok(
            json!({"path":path,"sha256":hash,"events":events.len(),"truncated":journal.len()>MAX_EVENTS as usize}),
        )
    }
}
