use super::*;
use std::path::{Path, PathBuf};
impl Database {
    async fn finish_run(&self, run: &str, status: &str, error: &str) -> Result<()> {
        sqlx::query("UPDATE execution_runs SET status=?,last_error=?,finished_at=? WHERE id=? AND owner_token=? AND status='RUNNING'").bind(status).bind(error).bind(now()).bind(run).bind(instance()).execute(&self.pool).await.map_err(|_|"EXECUTION_STATE_STORE_FAILED")?;
        Ok(())
    }
    pub(super) async fn drive(&self, run: &str, control: Arc<AtomicU8>) -> Result<()> {
        let claimed=sqlx::query("UPDATE execution_runs SET status='RUNNING',started_at=COALESCE(started_at,?) WHERE id=? AND status='QUEUED' AND owner_token=?").bind(now()).bind(run).bind(instance()).execute(&self.pool).await.map_err(|_|"EXECUTION_CLAIM_FAILED")?;
        if claimed.rows_affected() != 1 {
            return Ok(());
        }
        let row = sqlx::query("SELECT * FROM execution_runs WHERE id=?")
            .bind(run)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "DATABASE")?;
        let workflow: String = row.get("workflow_id");
        let plan = self.load_plan(&workflow, Some(row.get("revision"))).await?;
        let mode: String = row.get("mode");
        let workspace = PathBuf::from(row.get::<String, _>("workspace"));
        self.event(
            &workflow,
            Some(run),
            None,
            "RUNNING",
            json!({"mode":mode,"maxParallel":2}),
        )
        .await?;
        loop {
            match control.load(Ordering::SeqCst) {
                1 => {
                    self.finish_run(run, "RECOVERABLE", "PAUSED_EXPLICIT_RESUME_REQUIRED")
                        .await?;
                    self.event(
                        &workflow,
                        Some(run),
                        None,
                        "PAUSED",
                        json!({"automaticReplay":false}),
                    )
                    .await?;
                    return Ok(());
                }
                2 => {
                    self.finish_run(run, "CANCELLED", "USER_CANCELLED").await?;
                    self.event(
                        &workflow,
                        Some(run),
                        None,
                        "CANCELLED",
                        json!({"workerStopped":true}),
                    )
                    .await?;
                    return Ok(());
                }
                3 => {
                    self.finish_run(run, "FAILED", "NODE_TIMEOUT_OR_STORAGE_FAILURE")
                        .await?;
                    return Ok(());
                }
                _ => {}
            }
            if mode != "DRY_RUN" {
                let approved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_approvals WHERE execution_id=? AND plan_digest=? AND decision='APPROVED' AND expires_at>? AND id=(SELECT id FROM execution_approvals WHERE execution_id=? ORDER BY created_at DESC LIMIT 1))").bind(run).bind(plan.digest()).bind(now()).bind(run).fetch_one(&self.pool).await.map_err(|_|"APPROVAL_STORE_UNAVAILABLE")?;
                if !approved {
                    self.finish_run(run, "BLOCKED", "APPROVAL_MISSING_OR_EXPIRED")
                        .await?;
                    return Ok(());
                }
                if let Err(e) = self.source_check(&plan).await {
                    self.finish_run(run, "BLOCKED", &e).await?;
                    return Ok(());
                }
            }
            let history = self.run_steps(run).await?;
            let mut latest = BTreeMap::new();
            for step in &history {
                latest.insert(step["nodeId"].as_str().unwrap_or(""), step);
            }
            let handled: std::collections::BTreeSet<_> = plan
                .nodes
                .iter()
                .filter(|n| {
                    latest
                        .get(n.id.as_str())
                        .is_some_and(|s| s["status"] == "FAILED")
                        && plan.nodes.iter().any(|fallback| {
                            fallback.condition.as_ref().is_some_and(|c| {
                                c.node == n.id && c.field == "status" && c.equals == "FAILED"
                            }) && latest
                                .get(fallback.id.as_str())
                                .is_some_and(|s| s["status"] == "SUCCEEDED")
                        })
                })
                .map(|n| n.id.as_str())
                .collect();
            let retrying: std::collections::BTreeSet<_> = plan
                .nodes
                .iter()
                .filter(|n| {
                    latest.get(n.id.as_str()).is_some_and(|s| {
                        s["status"] == "FAILED"
                            && matches!(
                                s["errorType"].as_str(),
                                Some("FILE_UNREADABLE" | "RUNTIME_DISCONNECTED" | "RUNTIME_EXITED")
                            )
                    }) && n.retry_policy == "SAFE_READ_ONLY"
                        && history
                            .iter()
                            .filter(|s| s["nodeId"] == n.id && s["status"] == "FAILED")
                            .count()
                            < n.max_attempts as usize
                })
                .map(|n| n.id.as_str())
                .collect();
            if plan.nodes.iter().all(|n| {
                handled.contains(n.id.as_str())
                    || latest.get(n.id.as_str()).is_some_and(|s| {
                        matches!(s["status"].as_str(), Some("SUCCEEDED" | "SKIPPED"))
                    })
            }) {
                self.finish_run(run, "SUCCEEDED", "").await?;
                self.event(&workflow,Some(run),None,"SUCCEEDED",json!({"scope":plan.scope,"goalVerified":mode!="DRY_RUN"&&plan.goal_verifiable,"dryRun":mode=="DRY_RUN","mastery":"NOT_VERIFIED","handledFailureNodes":handled})).await?;
                return Ok(());
            }
            let mut ready = Vec::new();
            let mut skipped = Vec::new();
            for node in &plan.nodes {
                if let Some(s) = latest.get(node.id.as_str()) {
                    if matches!(
                        s["status"].as_str(),
                        Some("SUCCEEDED" | "SKIPPED" | "BLOCKED" | "CANCELLED")
                    ) {
                        continue;
                    }
                    if s["status"] == "RUNNING" {
                        return Err("ACTIVE_STEP_REQUIRES_CHECKPOINT_RECOVERY".into());
                    }
                    if s["status"] == "FAILED" {
                        let failures = history
                            .iter()
                            .filter(|h| h["nodeId"] == node.id && h["status"] == "FAILED")
                            .count();
                        if failures >= node.max_attempts as usize
                            || node.retry_policy != "SAFE_READ_ONLY"
                            || !matches!(
                                s["errorType"].as_str(),
                                Some("FILE_UNREADABLE" | "RUNTIME_DISCONNECTED" | "RUNTIME_EXITED")
                            )
                        {
                            continue;
                        }
                    }
                }
                let terminal = |id: &String| {
                    !retrying.contains(id.as_str())
                        && latest.get(id.as_str()).is_some_and(|s| {
                            matches!(
                                s["status"].as_str(),
                                Some("SUCCEEDED" | "SKIPPED" | "FAILED" | "BLOCKED")
                            )
                        })
                };
                if !node.dependencies.iter().all(terminal) {
                    continue;
                }
                let mut inputs = serde_json::Map::new();
                for d in &node.dependencies {
                    if let Some(s) = latest.get(d.as_str()) {
                        inputs.insert(d.clone(), s["output"].clone());
                    }
                }
                let mut allowed = true;
                if let Some(condition) = &node.condition {
                    let actual = if condition.field == "status" {
                        latest
                            .get(condition.node.as_str())
                            .map(|s| s["status"].clone())
                            .unwrap_or(Value::Null)
                    } else {
                        inputs
                            .get(&condition.node)
                            .map(|v| v[&condition.field].clone())
                            .unwrap_or(Value::Null)
                    };
                    if actual != condition.equals {
                        skipped.push((node.clone(),json!({"reason":"APPROVED_CONDITION_FALSE","condition":condition,"externalExecuted":false})));
                        continue;
                    }
                    allowed = node.dependencies.iter().all(|d| {
                        Some(d) == node.condition.as_ref().map(|c| &c.node)
                            || latest.get(d.as_str()).is_some_and(|s| {
                                matches!(s["status"].as_str(), Some("SUCCEEDED" | "SKIPPED"))
                            })
                    });
                } else if !node.dependencies.iter().all(|d| {
                    latest.get(d.as_str()).is_some_and(|s| {
                        matches!(s["status"].as_str(), Some("SUCCEEDED" | "SKIPPED"))
                    })
                }) {
                    allowed = false
                }
                if allowed {
                    ready.push((node.clone(), Value::Object(inputs)));
                }
            }
            if let Some((node, output)) = skipped.into_iter().next() {
                self.record_skipped(run, &plan, &node, &output).await?;
                continue;
            }
            if ready.is_empty() {
                let failed = history.iter().any(|s| s["status"] == "FAILED");
                self.finish_run(
                    run,
                    if failed { "FAILED" } else { "BLOCKED" },
                    if failed {
                        "STEP_FAILED_OR_RETRY_LIMIT_REACHED"
                    } else {
                        "DEPENDENCY_OR_ADAPTER_UNAVAILABLE"
                    },
                )
                .await?;
                self.event(
                    &workflow,
                    Some(run),
                    None,
                    "STOPPED",
                    json!({"error":"DEPENDENCY_OR_ADAPTER_UNAVAILABLE_OR_RETRY_EXHAUSTED"}),
                )
                .await?;
                return Ok(());
            }
            let mut jobs = Vec::new();
            for (node, inputs) in ready.into_iter().take(2) {
                let db = self.clone();
                let run = run.to_string();
                let plan = plan.clone();
                let workspace = workspace.clone();
                let mode = mode.clone();
                let control = control.clone();
                jobs.push(tauri::async_runtime::spawn(async move {
                    db.execute_node(&run, &plan, &node, &inputs, &workspace, &mode, control)
                        .await
                }));
            }
            for job in jobs {
                job.await
                    .map_err(|_| "WORKER_PANICKED_CHECKPOINT_PRESERVED")??;
            }
        }
    }
    async fn record_skipped(
        &self,
        run: &str,
        plan: &Plan,
        node: &Node,
        output: &Value,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO execution_steps(id,execution_id,node_id,attempt,status,idempotency_key,side_effect,input_digest,output_json,started_at,finished_at) VALUES(?,?,?,1,'SKIPPED',?,'NONE',?,?,?,?)").bind(id).bind(run).bind(&node.id).bind(format!("{run}:{}:skip",node.id)).bind(digest(node.id.as_bytes())).bind(output.to_string()).bind(now()).bind(now()).execute(&self.pool).await.map_err(|_|"STEP_STORE")?;
        self.event(
            &plan.workflow_id,
            Some(run),
            Some(&node.id),
            "SKIPPED",
            output.clone(),
        )
        .await
    }
    async fn execute_node(
        &self,
        run: &str,
        plan: &Plan,
        node: &Node,
        inputs: &Value,
        workspace: &Path,
        mode: &str,
        control: Arc<AtomicU8>,
    ) -> Result<()> {
        if control.load(Ordering::SeqCst) != 0 {
            return Ok(());
        }
        let adapter = adapters::resolve(&node.adapter)?;
        let prepared = adapter.prepare(node)?;
        let side_effect = prepared["sideEffect"].as_str().unwrap_or("NONE");
        let input_hash = digest(
            json!({"plan":plan.digest(),"node":node,"inputs":inputs})
                .to_string()
                .as_bytes(),
        );
        let id = uuid::Uuid::new_v4().to_string();
        let claim_guard = MUTATION.lock().await;
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_runs WHERE id=? AND status='RUNNING' AND owner_token=?)").bind(run).bind(instance()).fetch_one(&mut *tx).await.map_err(|_|"DATABASE")?;
        if !active || control.load(Ordering::SeqCst) != 0 {
            return Ok(());
        }
        let attempt:i64=sqlx::query_scalar("SELECT COALESCE(MAX(attempt),0)+1 FROM execution_steps WHERE execution_id=? AND node_id=?").bind(run).bind(&node.id).fetch_one(&mut *tx).await.map_err(|_|"DATABASE")?;
        if attempt > 10 {
            return Err("RECOVERY_ATTEMPT_LIMIT_REACHED".into());
        }
        let old: Option<(String, String)> = sqlx::query_as(
            "SELECT state,data_json FROM execution_checkpoints WHERE execution_id=? AND node_id=?",
        )
        .bind(run)
        .bind(&node.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| "DATABASE")?;
        if let Some((_, raw)) = old {
            let checkpoint: Value = serde_json::from_str(&raw).map_err(|_| "CHECKPOINT_INVALID")?;
            if checkpoint["inputDigest"] != input_hash {
                return Err("CHECKPOINT_INPUT_CHANGED_REPLAN_REQUIRED".into());
            }
        }
        sqlx::query("INSERT INTO execution_steps(id,execution_id,node_id,attempt,status,idempotency_key,side_effect,input_digest,started_at) VALUES(?,?,?,?,'RUNNING',?,?,?,?)").bind(&id).bind(run).bind(&node.id).bind(attempt).bind(format!("{run}:{}:{attempt}",node.id)).bind(if mode=="DRY_RUN"{"NONE"}else{side_effect}).bind(&input_hash).bind(now()).execute(&mut *tx).await.map_err(|_|"EXECUTION_CLAIM_FAILED")?;
        sqlx::query("INSERT INTO execution_checkpoints VALUES(?,?,'PREPARED',?,?) ON CONFLICT(execution_id,node_id) DO UPDATE SET state='PREPARED',updated_at=excluded.updated_at").bind(run).bind(&node.id).bind(json!({"inputDigest":input_hash,"sideEffectKey":format!("{run}:{}",node.id),"sideEffect":side_effect,"mode":mode}).to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|"CHECKPOINT_STORE")?;
        tx.commit().await.map_err(|_| "DATABASE")?;
        drop(claim_guard);
        self.event(
            &plan.workflow_id,
            Some(run),
            Some(&node.id),
            "NODE_RUNNING",
            json!({"adapter":node.adapter,"attempt":attempt}),
        )
        .await?;
        let result = if mode == "DRY_RUN" {
            Ok(
                json!({"dryRun":true,"executionOccurred":false,"preparation":prepared,"scope":plan.scope}),
            )
        } else if prepared["ready"] != true {
            Err(prepared["reason"]
                .as_str()
                .unwrap_or("ADAPTER_UNSUPPORTED")
                .into())
        } else {
            let (progress, mut progress_events) = tokio::sync::mpsc::unbounded_channel();
            let context = adapters::Context {
                plan,
                node,
                inputs,
                workspace,
                control: control.clone(),
                progress,
            };
            let execute = async {
                let adapter_work = adapter.execute(context);
                tokio::pin!(adapter_work);
                let mut count = 0;
                loop {
                    tokio::select! {
                        result=&mut adapter_work=>{
                            while let Ok(event)=progress_events.try_recv(){
                                self.event(&plan.workflow_id,Some(run),Some(&node.id),event,json!({"adapter":node.adapter})).await?;
                            }
                            break result
                        },
                        Some(event)=progress_events.recv()=>{
                            count+=1;if count>32{control.store(3,Ordering::SeqCst);break Err("ADAPTER_PROGRESS_LIMIT".into())}
                            if let Err(e)=self.event(&plan.workflow_id,Some(run),Some(&node.id),event,json!({"adapter":node.adapter})).await {control.store(3,Ordering::SeqCst);break Err(e)}
                        }
                    }
                }
            };
            match tokio::time::timeout(std::time::Duration::from_millis(node.timeout_ms), execute)
                .await
            {
                Ok(value) => value,
                Err(_) => {
                    control.store(3, Ordering::SeqCst);
                    Err("NODE_TIMEOUT".into())
                }
            }
        };
        let (status, output, error) = match result {
            Ok(output) if control.load(Ordering::SeqCst) == 0 => {
                ("SUCCEEDED", Some(output), String::new())
            }
            Ok(_) => ("INTERRUPTED", None, "CANCELLED_OR_PAUSED".into()),
            Err(e) => {
                let state = if control.load(Ordering::SeqCst) == 2 {
                    "CANCELLED"
                } else if control.load(Ordering::SeqCst) == 1 {
                    "INTERRUPTED"
                } else if e.contains("NOT_VERIFIED")
                    || e.contains("UNSUPPORTED")
                    || e.contains("NOT_CONNECTED")
                {
                    "BLOCKED"
                } else {
                    "FAILED"
                };
                (state, None, clean(&e))
            }
        };
        let proof = if let Some(output) = &output {
            if mode == "DRY_RUN" {
                None
            } else {
                let bytes = adapter.collect_evidence(node, output)?;
                let path = workspace
                    .join("evidence")
                    .join(format!("{}-{attempt}.json", node.id));
                let hash = security::seal(&path, &bytes)?;
                Some((path, hash, bytes.len()))
            }
        } else {
            None
        };
        let _commit_guard = MUTATION.lock().await;
        let mut tx = self.pool.begin().await.map_err(|_| "DATABASE")?;
        sqlx::query("UPDATE execution_steps SET status=?,output_digest=?,output_json=?,error_type=?,finished_at=? WHERE id=? AND status='RUNNING'").bind(status).bind(output.as_ref().map(|o|digest(o.to_string().as_bytes()))).bind(output.as_ref().map(Value::to_string)).bind(&error).bind(now()).bind(&id).execute(&mut *tx).await.map_err(|_|"STEP_COMMIT_FAILED_CHECKPOINT_RETAINED")?;
        if let Some((path, hash, size)) = proof {
            sqlx::query("INSERT INTO execution_evidence VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(execution_id,node_id) DO NOTHING").bind(uuid::Uuid::new_v4().to_string()).bind(run).bind(&node.id).bind(&node.capability_id).bind(&node.capability_version).bind(&node.adapter).bind(path.to_string_lossy().as_ref()).bind(hash).bind(size as i64).bind(&node.verification_policy).bind(now()).execute(&mut *tx).await.map_err(|_|"EVIDENCE_COMMIT")?;
        }
        sqlx::query("UPDATE execution_checkpoints SET state=?,updated_at=? WHERE execution_id=? AND node_id=?").bind(if status=="SUCCEEDED"{"COMMITTED"}else{"REQUIRES_REVIEW"}).bind(now()).bind(run).bind(&node.id).execute(&mut *tx).await.map_err(|_|"CHECKPOINT_STORE")?;
        sqlx::query("INSERT INTO execution_events(workflow_id,execution_id,node_id,event,data_json,created_at) VALUES(?,?,?,?,?,?)").bind(&plan.workflow_id).bind(run).bind(&node.id).bind(status).bind(json!({"adapter":node.adapter,"errorType":error,"dryRun":mode=="DRY_RUN"}).to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|"EVENT_STORE_UNAVAILABLE")?;
        tx.commit().await.map_err(|_| "DATABASE")?;
        publish(
            json!({"workflowId":plan.workflow_id,"executionId":run,"nodeId":node.id,"event":status}),
        );
        Ok(())
    }
}
