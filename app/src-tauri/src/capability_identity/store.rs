use super::*;
use crate::db::Database;
use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::collections::{BTreeMap, BTreeSet};
const MIGRATION: &str = include_str!("../../migrations/0007_logical_capability_identities.sql");
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRef {
    pub source_database: PathBuf,
    pub backup: PathBuf,
    pub restored: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub schema: i64,
    pub schema_hash: String,
    pub tables: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationPlan {
    pub context: ScanContext,
    pub backup: BackupRef,
    pub baseline: Fingerprint,
    pub snapshot_fingerprint: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub report: CodexScanReport,
    pub historical: Vec<Observation>,
    pub new_entity_ids: Vec<String>,
    pub absent_entity_ids: Vec<String>,
    pub refreshed_entity_ids: Vec<String>,
    pub unresolved_entity_ids: Vec<String>,
    pub expected: Value,
    pub warnings: Vec<String>,
    pub can_apply: bool,
}
impl ReconciliationPlan {
    pub fn digest(&self) -> String {
        hash(&serde_json::to_string(self).unwrap())
    }
}
pub(crate) async fn fingerprint_conn(conn: &mut SqliteConnection) -> Result<Fingerprint> {
    let schema: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| "Cannot read schema version.")?;
    let definitions: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
            .fetch_all(&mut *conn)
            .await
            .map_err(|_| "Cannot read schema definitions.")?;
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(&mut *conn)
            .await
            .map_err(|_| "Cannot list database tables.")?;
    let mut tables = BTreeMap::new();
    for name in names {
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("Unexpected database table identifier.".into());
        }
        let info = sqlx::query(&format!("PRAGMA table_info(\"{name}\")"))
            .fetch_all(&mut *conn)
            .await
            .map_err(|_| "Cannot read columns.")?;
        let columns: Vec<String> = info
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
        if columns
            .iter()
            .any(|c| !c.chars().all(|v| v.is_ascii_alphanumeric() || v == '_'))
        {
            return Err("Unexpected column identifier.".into());
        }
        let expression = columns
            .iter()
            .map(|c| format!("quote(\"{c}\")"))
            .collect::<Vec<_>>()
            .join("||char(31)||");
        let mut rows: Vec<String> =
            sqlx::query_scalar(&format!("SELECT {expression} FROM \"{name}\" "))
                .fetch_all(&mut *conn)
                .await
                .map_err(|_| "Cannot fingerprint table contents.")?;
        rows.sort();
        tables.insert(
            name,
            json!({"rows":rows.len(),"sha256":hash(&serde_json::to_string(&rows).unwrap())}),
        );
    }
    Ok(Fingerprint {
        schema,
        schema_hash: hash(&serde_json::to_string(&definitions).unwrap()),
        tables,
    })
}
pub async fn fingerprint(pool: &SqlitePool) -> Result<Fingerprint> {
    let mut c = pool
        .acquire()
        .await
        .map_err(|_| "Database is unavailable.")?;
    fingerprint_conn(&mut c).await
}
pub async fn schema_ready(conn: &mut SqliteConnection) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='logical_capabilities')").fetch_one(conn).await.map_err(|_|"Cannot inspect identity schema.".into())
}
impl Database {
    pub async fn require_identity_scan_ready(&self) -> Result<()> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|_| "Database is unavailable.")?;
        if !schema_ready(&mut connection).await? {
            return Err("Identity reconciliation requires a verified backup, preview and explicit approval before persisting a new scan. The existing database was preserved.".into());
        }
        Ok(())
    }
    pub async fn verify_identity_backup(&self, backup: &BackupRef) -> Result<Fingerprint> {
        if normalized(self.path()) != normalized(&backup.source_database) {
            return Err("Backup belongs to another database.".into());
        }
        let current = fingerprint(&self.pool).await?;
        for p in [&backup.backup, &backup.restored] {
            if !p.is_absolute()
                || normalized(p) == normalized(self.path())
                || !ordinary(p)
                || (cfg!(windows) && !normalized(p).starts_with("d:/"))
            {
                return Err(
                    "Backup/restore verification must use independent ordinary files on D:.".into(),
                );
            }
            let db = Database::open_read_only(p.clone()).await?;
            db.integrity_check().await?;
            let foreign = sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(&db.pool)
                .await
                .map_err(|_| "Cannot validate restored foreign keys.")?;
            let observed = fingerprint(&db.pool).await?;
            db.pool.close().await;
            if !foreign.is_empty() || observed != current {
                return Err(
                    "A complete backup/independent restore does not match the current database."
                        .into(),
                );
            }
        }
        if normalized(&backup.backup) == normalized(&backup.restored) {
            return Err("Restore validation requires a distinct file.".into());
        }
        Ok(current)
    }
    pub async fn preview_identity_reconciliation(
        &self,
        context: ScanContext,
        backup: BackupRef,
    ) -> Result<ReconciliationPlan> {
        let baseline = self.verify_identity_backup(&backup).await?;
        let report = scan(&context)?;
        let identity = report
            .identity
            .as_ref()
            .ok_or("Identity scan unavailable.")?;
        let old = self.load_codex_scan().await?;
        let proven = old
            .as_ref()
            .map(|old| historical(&report, old))
            .unwrap_or_default();
        let stored:Vec<(String,String,String)>=sqlx::query_as("SELECT e.id,i.status,e.metadata_json FROM entities e JOIN installations i ON i.entity_id=e.id WHERE e.source_type='local' AND i.source='scanner' ORDER BY e.id").fetch_all(&self.pool).await.map_err(|_|"Cannot read historical local records.")?;
        let stored_ids: BTreeSet<_> = stored.iter().map(|(id, _, _)| id.clone()).collect();
        let current_ids: BTreeSet<_> = identity
            .observations
            .iter()
            .map(|o| o.entity_id.clone())
            .collect();
        let mut history = Vec::new();
        let mut unresolved = Vec::new();
        let mut warnings = identity.warnings.clone();
        for (id, status, metadata) in stored.iter().filter(|(id, _, _)| !current_ids.contains(id)) {
            let prior: Value = serde_json::from_str(metadata).unwrap_or(Value::Null);
            if prior["scopeKey"]
                .as_str()
                .is_some_and(|scope| !context.scopes().iter().any(|s| s == scope))
            {
                continue;
            }
            let known = if let Some(o) = proven.get(id) {
                Some(o.clone())
            } else {
                self.identity_observation_for_entity(id).await?
            };
            let Some(mut o) = known else {
                if status != "NOT_INSTALLED" {
                    unresolved.push(id.clone());
                }
                continue;
            };
            let inspected = if o.kind == "mcp" {
                o.manifest_path.as_ref().unwrap_or(&o.location)
            } else {
                &o.location
            };
            match fs::symlink_metadata(inspected) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    o.present = false;
                    o.status = "NOT_INSTALLED".into();
                    history.push(o);
                }
                Ok(_) => {
                    if status != "NOT_INSTALLED" {
                        unresolved.push(id.clone());
                    }
                }
                Err(_) => {
                    unresolved.push(id.clone());
                    warnings.push("A historical location cannot be checked; absence is not inferred from access errors.".into());
                }
            }
        }
        history.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        let now = crate::github::types::now();
        let new_ids = current_ids
            .difference(&stored_ids)
            .cloned()
            .collect::<Vec<_>>();
        let refresh = current_ids
            .intersection(&stored_ids)
            .cloned()
            .collect::<Vec<_>>();
        let absent = history
            .iter()
            .map(|o| o.entity_id.clone())
            .collect::<Vec<_>>();
        let source_entities: i64 =
            sqlx::query_scalar("SELECT count(*) FROM entities WHERE source_type='github'")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "Cannot count discovery entities.")?;
        let visible_candidates: i64 =
            sqlx::query_scalar("SELECT count(*) FROM github_capability_sources")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "Cannot count discovery candidates.")?;
        let expected = json!({"entities":baseline.tables.get("entities").and_then(|v|v["rows"].as_u64()).unwrap_or(0)+new_ids.len() as u64,"legacyInstallations":baseline.tables.get("installations").and_then(|v|v["rows"].as_u64()).unwrap_or(0)+new_ids.len() as u64,"currentPhysicalFindings":identity.observations.len(),"currentLogicalCapabilities":identity.observations.iter().map(|o|&o.logical_id).collect::<BTreeSet<_>>().len(),"discoverySourceEntities":source_entities,"discoveryCandidates":visible_candidates,"historicalAbsent":history.len()});
        if !history.is_empty() {
            warnings.push("Historical links use exact old path IDs, saved valid manifest metadata and a uniquely verified current source component; old manifest bytes are unavailable. These links require this reconciliation approval.".into());
        }
        Ok(ReconciliationPlan {
            context,
            backup,
            baseline,
            snapshot_fingerprint: identity.fingerprint(),
            created_at: now,
            expires_at: now + 60 * 60_000,
            report,
            historical: history,
            new_entity_ids: new_ids,
            absent_entity_ids: absent,
            refreshed_entity_ids: refresh,
            unresolved_entity_ids: unresolved.clone(),
            expected,
            warnings,
            can_apply: unresolved.is_empty(),
        })
    }
    pub async fn apply_identity_reconciliation(
        &self,
        plan: &ReconciliationPlan,
        expected_digest: &str,
        confirmed: bool,
    ) -> Result<()> {
        if !confirmed || expected_digest != plan.digest() {
            return Err("Confirm the exact reconciliation preview before database writes.".into());
        }
        if !plan.can_apply
            || !plan.unresolved_entity_ids.is_empty()
            || crate::github::types::now() > plan.expires_at
        {
            return Err("The preview is blocked or expired; no reconciliation was applied.".into());
        }
        let fresh = self
            .preview_identity_reconciliation(plan.context.clone(), plan.backup.clone())
            .await?;
        if fresh.baseline != plan.baseline
            || fresh.snapshot_fingerprint != plan.snapshot_fingerprint
            || fresh.new_entity_ids != plan.new_entity_ids
            || fresh.absent_entity_ids != plan.absent_entity_ids
        {
            return Err(
                "Database or source evidence changed after preview; a new approval is required."
                    .into(),
            );
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "Cannot begin reconciliation.")?;
        if fingerprint_conn(&mut tx).await? != plan.baseline {
            return Err("Database changed while obtaining the reconciliation transaction.".into());
        }
        let ready = schema_ready(&mut tx).await?;
        if !ready {
            for statement in MIGRATION.split(';').filter(|s| !s.trim().is_empty()) {
                sqlx::query(statement)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| "Identity migration failed; transaction will roll back.")?;
            }
        }
        if !ready {
            if let Some(raw) = self_scan_before_in(&mut tx).await? {
                let old: Value =
                    serde_json::from_str(&raw).map_err(|_| "Invalid original scan snapshot.")?;
                let at = old["scannedAtUnixMs"]
                    .as_u64()
                    .ok_or("Original scan timestamp is missing.")?;
                let context = json!({"recordedScanContext":old["identity"]["context"],"contextProvenance":if old["identity"]["context"].is_object(){"RECORDED"}else{"NOT_RECORDED"},"reconciliationContext":plan.context});
                // Retain the original serialized snapshot, including unknown historical fields.
                // A reconciliation context is not proof of the earlier scan's context.
                sqlx::query("INSERT OR IGNORE INTO capability_scan_history(scan_id,scanned_at,context_json,report_json,imported) VALUES(?,?,?,?,1)").bind(format!("legacy:{}",hash(&raw))).bind(at as i64).bind(context.to_string()).bind(raw).execute(&mut *tx).await.map_err(|_|"Cannot retain original scan snapshot.")?;
            }
        }
        for o in &plan.historical {
            save_observation(&mut tx, o, plan.created_at, None).await?;
            mark_absent(&mut tx, o, plan.created_at, None, true).await?;
        }
        let mut current_report = fresh.report;
        current_report.scanned_at_unix_ms = plan.report.scanned_at_unix_ms;
        self.persist_codex_scan_in(&mut tx, &current_report).await?;
        sqlx::query("INSERT INTO capability_reconciliations(plan_digest,approved_at,backup_json,preview_json) VALUES(?,?,?,?)").bind(expected_digest).bind(crate::github::types::now()).bind(json!(plan.backup).to_string()).bind(json!(plan).to_string()).execute(&mut *tx).await.map_err(|_|"Cannot save reconciliation approval audit.")?;
        tx.commit()
            .await
            .map_err(|_| "Reconciliation commit failed.")?;
        Ok(())
    }
    async fn identity_observation_for_entity(&self, id: &str) -> Result<Option<Observation>> {
        let mut c = self
            .pool
            .acquire()
            .await
            .map_err(|_| "Database unavailable.")?;
        if !schema_ready(&mut c).await? {
            return Ok(None);
        }
        let raw:Option<String>=sqlx::query_scalar("SELECT evidence_json FROM capability_physical_installations WHERE entity_id=? ORDER BY last_seen DESC LIMIT 1").bind(id).fetch_optional(&mut *c).await.map_err(|_|"Cannot read retained installation evidence.")?;
        raw.map(|s| {
            serde_json::from_str(&s).map_err(|_| "Invalid retained installation evidence.".into())
        })
        .transpose()
    }
    pub async fn identity_history(&self, scopes: &[String]) -> Result<Vec<Observation>> {
        let mut c = self
            .pool
            .acquire()
            .await
            .map_err(|_| "Database unavailable.")?;
        if !schema_ready(&mut c).await? {
            return Ok(vec![]);
        }
        let rows:Vec<String>=sqlx::query_scalar("SELECT evidence_json FROM capability_physical_installations WHERE scope_key IN (SELECT value FROM json_each(?)) ORDER BY first_seen,physical_id").bind(json!(scopes).to_string()).fetch_all(&mut *c).await.map_err(|_|"Cannot load installation history.")?;
        rows.iter()
            .map(|s| serde_json::from_str(s).map_err(|_| "Invalid installation history.".into()))
            .collect()
    }
}
async fn self_scan_before_in(tx: &mut Transaction<'_, Sqlite>) -> Result<Option<String>> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT report_json FROM codex_scan_state WHERE id='latest'")
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| "Cannot preserve old scan.")?;
    Ok(raw)
}
async fn save_observation(
    tx: &mut Transaction<'_, Sqlite>,
    o: &Observation,
    at: i64,
    operation: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO capability_origins(origin_key,kind,authority_json,created_operation_id) VALUES(?,?,?,?) ON CONFLICT(origin_key) DO NOTHING").bind(&o.origin_key).bind(&o.kind).bind(o.authority.to_string()).bind(operation).execute(&mut **tx).await.map_err(|_|"Cannot record discovery provenance.")?;
    sqlx::query("INSERT INTO logical_capabilities(logical_id,kind,machine_name,origin_key,component_key,confidence,created_at,created_operation_id) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(logical_id) DO NOTHING").bind(&o.logical_id).bind(&o.kind).bind(&o.machine_name).bind(&o.origin_key).bind(&o.component_key).bind(&o.confidence).bind(at).bind(operation).execute(&mut **tx).await.map_err(|_|"Cannot record logical identity.")?;
    sqlx::query("INSERT INTO capability_entity_links(entity_id,logical_id,proof_json,last_operation_id) VALUES(?,?,?,?) ON CONFLICT(entity_id) DO UPDATE SET logical_id=excluded.logical_id,proof_json=excluded.proof_json,last_operation_id=excluded.last_operation_id").bind(&o.entity_id).bind(&o.logical_id).bind(o.evidence.to_string()).bind(operation).execute(&mut **tx).await.map_err(|_|"Cannot link historical physical entity.")?;
    sqlx::query("INSERT INTO capability_physical_installations(physical_id,entity_id,logical_id,scope_key,location,manifest_path,package_version,cache_version,manifest_hash,present,status,first_seen,last_seen,evidence_json,last_operation_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(physical_id) DO UPDATE SET present=excluded.present,status=excluded.status,last_seen=excluded.last_seen,evidence_json=excluded.evidence_json,last_operation_id=excluded.last_operation_id").bind(&o.physical_id).bind(&o.entity_id).bind(&o.logical_id).bind(&o.scope_key).bind(normalized(&o.location)).bind(o.manifest_path.as_ref().map(|p|normalized(p))).bind(&o.package_version).bind(&o.cache_version).bind(&o.manifest_hash).bind(o.present as i64).bind(&o.status).bind(at).bind(at).bind(json!(o).to_string()).bind(operation).execute(&mut **tx).await.map_err(|_|"Cannot record version/location observation.")?;
    Ok(())
}
async fn mark_absent(
    tx: &mut Transaction<'_, Sqlite>,
    o: &Observation,
    at: i64,
    operation: Option<&str>,
    legacy_absent: bool,
) -> Result<()> {
    sqlx::query("UPDATE capability_physical_installations SET present=0,status='NOT_INSTALLED',last_seen=?,evidence_json=?,last_operation_id=? WHERE physical_id=?").bind(at).bind(json!(o).to_string()).bind(operation).bind(&o.physical_id).execute(&mut **tx).await.map_err(|_|"Cannot retain missing physical installation.")?;
    if legacy_absent {
        sqlx::query("UPDATE installations SET status='NOT_INSTALLED',observed_at=?,evidence_json=json_patch(evidence_json,?),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE source='scanner' AND entity_id=?").bind(at.to_string()).bind(json!({"scannerObserved":false,"reason":"physical location/version no longer observed","logicalId":o.logical_id,"physicalId":o.physical_id}).to_string()).bind(&o.entity_id).execute(&mut **tx).await.map_err(|_|"Cannot retain historical status.")?;
    }
    Ok(())
}
pub async fn sync_in(tx: &mut Transaction<'_, Sqlite>, report: &CodexScanReport) -> Result<()> {
    let Some(scan) = report.identity.as_ref() else {
        return Ok(());
    };
    if !schema_ready(tx).await? {
        return Ok(());
    }
    let at = report.scanned_at_unix_ms as i64;
    let scan_id = format!(
        "scan:{}",
        hash(&json!([scan.context, at, scan.fingerprint()]).to_string())
    );
    let operation = scan.operation_id.as_deref().or_else(|| {
        report
            .entities
            .iter()
            .find_map(|e| e.evidence["installationOperationId"].as_str())
    });
    sqlx::query("INSERT OR IGNORE INTO capability_scan_history(scan_id,scanned_at,context_json,report_json,operation_id) VALUES(?,?,?,?,?)").bind(&scan_id).bind(at).bind(json!(scan.context).to_string()).bind(json!(report).to_string()).bind(operation).execute(&mut **tx).await.map_err(|_|"Cannot preserve scan history.")?;
    let previous:Vec<String>=sqlx::query_scalar("SELECT evidence_json FROM capability_physical_installations WHERE present=1 AND scope_key IN (SELECT value FROM json_each(?))").bind(json!(scan.context.scopes()).to_string()).fetch_all(&mut **tx).await.map_err(|_|"Cannot read previous physical observations.")?;
    for raw in previous {
        let mut old: Observation =
            serde_json::from_str(&raw).map_err(|_| "Invalid physical observation.")?;
        if scan
            .observations
            .iter()
            .any(|o| o.physical_id == old.physical_id)
        {
            continue;
        }
        let replaced = scan.observations.iter().any(|o| {
            normalized(&o.location) == normalized(&old.location) && o.entity_id == old.entity_id
        });
        let installation_evidence = old.manifest_path.as_ref().unwrap_or(&old.location);
        let absent = matches!(fs::symlink_metadata(installation_evidence),Err(e) if e.kind()==std::io::ErrorKind::NotFound);
        let removed_config_entry = old.kind == "mcp"
            && scan
                .observations
                .iter()
                .all(|o| o.entity_id != old.entity_id)
            && read(&old.location, 1024 * 1024)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
                .is_some_and(|v| {
                    v.get("mcp_servers")
                        .and_then(|s| s.get(&old.machine_name))
                        .is_none()
                });
        if replaced || absent || removed_config_entry {
            old.present = false;
            old.status = "NOT_INSTALLED".into();
            // A new revision may share this legacy path alias; only retire the
            // old physical record when that alias is still present in this scan.
            let legacy_absent = !scan
                .observations
                .iter()
                .any(|o| o.entity_id == old.entity_id);
            mark_absent(tx, &old, at, operation, legacy_absent).await?;
        }
    }
    for o in &scan.observations {
        save_observation(tx, o, at, operation).await?;
        sqlx::query("INSERT OR IGNORE INTO capability_scan_observations(scan_id,physical_id,entity_id,status) VALUES(?,?,?,?)").bind(&scan_id).bind(&o.physical_id).bind(&o.entity_id).bind(&o.status).execute(&mut **tx).await.map_err(|_|"Cannot retain scan observation.")?;
    }
    Ok(())
}
