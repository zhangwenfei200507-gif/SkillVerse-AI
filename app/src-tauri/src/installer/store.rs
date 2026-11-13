use super::types::*;
use crate::db::Database;
use serde_json::{Value, json};

async fn snapshot_rows(
    pool: &sqlx::SqlitePool,
    table: &str,
    filter: &str,
    entity: &str,
) -> Result<Value> {
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await
            .map_err(|_| Error::database())?;
    let args = columns
        .iter()
        .map(|(_, name, _, _, _, _)| format!("'{name}',\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    let query = format!("SELECT json_object({args}) FROM {table} WHERE {filter}");
    let rows: Vec<String> = sqlx::query_scalar(&query)
        .bind(entity)
        .fetch_all(pool)
        .await
        .map_err(|_| Error::database())?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).unwrap_or(Value::Null))
            .collect(),
    ))
}
async fn restore_key_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    primary_key: &str,
    row: &Value,
) -> Result<()> {
    let object = row
        .as_object()
        .ok_or_else(|| fail("BACKUP", "Database snapshot row is invalid."))?;
    if object
        .keys()
        .any(|key| !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Err(fail("BACKUP", "Database snapshot columns are invalid."));
    }
    let names = object
        .keys()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    let placeholders = vec!["?"; object.len()].join(",");
    let update = object
        .keys()
        .filter(|key| key.as_str() != primary_key)
        .map(|key| format!("\"{key}\"=excluded.\"{key}\""))
        .collect::<Vec<_>>()
        .join(",");
    let text = format!(
        "INSERT INTO {table}({names}) VALUES({placeholders}) ON CONFLICT({primary_key}) DO UPDATE SET {update}"
    );
    let mut query = sqlx::query(&text);
    for value in object.values() {
        query = match value {
            Value::Null => query.bind(Option::<String>::None),
            Value::Number(n) => query.bind(
                n.as_i64()
                    .ok_or_else(|| fail("BACKUP", "Snapshot numeric value is invalid."))?,
            ),
            Value::String(s) => query.bind(s),
            _ => return Err(fail("BACKUP", "Snapshot cell type is invalid.")),
        };
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|_| Error::database())?;
    Ok(())
}
async fn restore_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    row: &Value,
) -> Result<()> {
    restore_key_row(tx, table, "id", row).await
}
impl Database {
    pub(crate) async fn install_database_snapshot(&self, entity: &str) -> Result<Value> {
        let mut result = json!({"entities":snapshot_rows(&self.pool,"entities","source_type='local' AND ? IS NOT NULL",entity).await?,"scannerInstallations":snapshot_rows(&self.pool,"installations","source='scanner' AND ? IS NOT NULL",entity).await?,"appInstallation":snapshot_rows(&self.pool,"installations","entity_id=?",entity).await?,"scan":snapshot_rows(&self.pool,"codex_scan_state","? IS NOT NULL",entity).await?});
        let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='capability_physical_installations')").fetch_one(&self.pool).await.map_err(|_|Error::database())?;
        if ready {
            for table in [
                "capability_physical_installations",
                "capability_entity_links",
            ] {
                result["identity"][table] =
                    snapshot_rows(&self.pool, table, "? IS NOT NULL", entity).await?;
            }
        }
        Ok(result)
    }
    async fn install_restore_identity_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        op: &Operation,
        before: &Value,
    ) -> Result<()> {
        if !before["identity"].is_object() {
            return Ok(());
        }
        sqlx::query("DELETE FROM capability_scan_observations WHERE scan_id IN (SELECT scan_id FROM capability_scan_history WHERE operation_id=?)").bind(&op.id).execute(&mut **tx).await.map_err(|_|Error::database())?;
        sqlx::query("DELETE FROM capability_scan_history WHERE operation_id=?")
            .bind(&op.id)
            .execute(&mut **tx)
            .await
            .map_err(|_| Error::database())?;
        for (table, key) in [
            ("capability_physical_installations", "physical_id"),
            ("capability_entity_links", "entity_id"),
        ] {
            let keys: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT {key} FROM {table} WHERE last_operation_id=?"
            ))
            .bind(&op.id)
            .fetch_all(&mut **tx)
            .await
            .map_err(|_| Error::database())?;
            for value in keys {
                if let Some(row) = before["identity"][table]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row[key] == value))
                {
                    restore_key_row(tx, table, key, row).await?;
                } else {
                    sqlx::query(&format!(
                        "DELETE FROM {table} WHERE {key}=? AND last_operation_id=?"
                    ))
                    .bind(&value)
                    .bind(&op.id)
                    .execute(&mut **tx)
                    .await
                    .map_err(|_| Error::database())?;
                }
            }
        }
        sqlx::query("DELETE FROM logical_capabilities WHERE created_operation_id=? AND logical_id NOT IN (SELECT logical_id FROM capability_physical_installations) AND logical_id NOT IN (SELECT logical_id FROM capability_entity_links)").bind(&op.id).execute(&mut **tx).await.map_err(|_|Error::database())?;
        sqlx::query("DELETE FROM capability_origins WHERE created_operation_id=? AND origin_key NOT IN (SELECT origin_key FROM logical_capabilities)").bind(&op.id).execute(&mut **tx).await.map_err(|_|Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_restore_database(
        &self,
        op: &Operation,
        before: &Value,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::database())?;
        Self::install_restore_identity_in(&mut tx, op, before).await?;
        let marked:Vec<String>=sqlx::query_scalar("SELECT entity_id FROM installations WHERE source='scanner' AND json_extract(evidence_json,'$.installationOperationId')=?").bind(&op.id).fetch_all(&mut *tx).await.map_err(|_|Error::database())?;
        for id in marked {
            if let Some(row) = before["entities"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["id"] == id))
            {
                restore_row(&mut tx, "entities", row).await?;
                if let Some(install) = before["scannerInstallations"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row["entity_id"] == id))
                {
                    restore_row(&mut tx, "installations", install).await?;
                }
            } else {
                let references:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM learning_records WHERE entity_id=?)+(SELECT count(*) FROM github_classifications WHERE entity_id=?)+(SELECT count(*) FROM github_local_links WHERE local_entity_id=?)").bind(&id).bind(&id).bind(&id).fetch_one(&mut *tx).await.map_err(|_|Error::database())?;
                if references == 0 {
                    sqlx::query("DELETE FROM graph_nodes WHERE entity_id=?")
                        .bind(&id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|_| Error::database())?;
                    sqlx::query("DELETE FROM installations WHERE entity_id=?")
                        .bind(&id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|_| Error::database())?;
                    sqlx::query("DELETE FROM entities WHERE id=? AND source_type='local'")
                        .bind(&id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|_| Error::database())?;
                } else {
                    sqlx::query(
                        "UPDATE installations SET status='NOT_INSTALLED' WHERE entity_id=?",
                    )
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::database())?;
                }
            }
        }
        let current_scan: Option<i64> =
            sqlx::query_scalar("SELECT scanned_at_unix_ms FROM codex_scan_state WHERE id='latest'")
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| Error::database())?;
        if current_scan == op.recovery["scanAfterAt"].as_i64() {
            if let Some(row) = before["scan"].as_array().and_then(|rows| rows.first()) {
                restore_row(&mut tx, "codex_scan_state", row).await?;
            } else {
                sqlx::query("DELETE FROM codex_scan_state WHERE id='latest'")
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::database())?;
            }
        }
        let app_owner:Option<String>=sqlx::query_scalar("SELECT json_extract(evidence_json,'$.operationId') FROM installations WHERE entity_id=? AND source='app'").bind(&op.plan.entity_id).fetch_optional(&mut *tx).await.map_err(|_|Error::database())?;
        if app_owner.as_deref() == Some(&op.id) {
            if let Some(row) = before["appInstallation"]
                .as_array()
                .and_then(|rows| rows.first())
            {
                restore_row(&mut tx, "installations", row).await?;
            } else {
                sqlx::query("DELETE FROM installations WHERE entity_id=? AND source='app'")
                    .bind(&op.plan.entity_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::database())?;
            }
        }
        if let Some(record) = op.plan.prior_record.as_ref() {
            sqlx::query("UPDATE managed_installations SET record_json=?,state=?,current_operation_id=?,updated_at=? WHERE id=? AND current_operation_id=?").bind(json!(record).to_string()).bind(&record.state).bind(&record.current_operation_id).bind(time()).bind(&record.id).bind(&op.id).execute(&mut *tx).await.map_err(|_|Error::database())?;
        } else {
            sqlx::query("DELETE FROM managed_installations WHERE current_operation_id=?")
                .bind(&op.id)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::database())?;
        }
        tx.commit().await.map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_plan(&self, plan: &Plan) -> Result<Operation> {
        let raw = serde_json::to_string(plan)
            .map_err(|_| fail("PLAN", "The installation plan cannot be encoded."))?;
        let digest = hash(raw.as_bytes());
        let now = time();
        let mut tx = self.pool.begin().await.map_err(|_| Error::database())?;
        sqlx::query("INSERT INTO installation_operations(id,profile_id,entity_id,action,state,plan_json,plan_digest,created_at,updated_at) VALUES(?,?,?,?,'WAITING_FOR_APPROVAL',?,?,?,?)").bind(&plan.operation_id).bind(&plan.profile.id).bind(&plan.entity_id).bind(&plan.action).bind(&raw).bind(&digest).bind(now).bind(now).execute(&mut *tx).await.map_err(|_|Error::database())?;
        for state in ["PLANNED", "WAITING_FOR_APPROVAL"] {
            sqlx::query("INSERT INTO installation_operation_events(operation_id,state,details_json,created_at) VALUES(?,?,'{}',?)").bind(&plan.operation_id).bind(state).bind(now).execute(&mut *tx).await.map_err(|_|Error::database())?;
        }
        tx.commit().await.map_err(|_| Error::database())?;
        self.install_operation(&plan.operation_id).await
    }
    pub(crate) async fn install_operation(&self, id: &str) -> Result<Operation> {
        let row:(String,String,String,Option<String>,i64,String)=sqlx::query_as("SELECT state,plan_json,plan_digest,error_json,updated_at,recovery_json FROM installation_operations WHERE id=?").bind(id).fetch_optional(&self.pool).await.map_err(|_|Error::database())?.ok_or_else(||fail("NOT_FOUND","Installation operation was not found."))?;
        Ok(Operation {
            id: id.into(),
            state: row.0,
            plan: serde_json::from_str(&row.1)
                .map_err(|_| fail("JOURNAL", "The saved plan is invalid."))?,
            digest: row.2,
            error: row.3.and_then(|v| serde_json::from_str(&v).ok()),
            updated_at: row.4,
            recovery: serde_json::from_str(&row.5)
                .map_err(|_| fail("JOURNAL", "Recovery metadata is invalid."))?,
        })
    }
    pub(crate) async fn install_event(&self, id: &str, state: &str, details: &Value) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::database())?;
        let now = time();
        sqlx::query("UPDATE installation_operations SET state=?,updated_at=? WHERE id=?")
            .bind(state)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::database())?;
        sqlx::query("INSERT INTO installation_operation_events(operation_id,state,details_json,created_at) VALUES(?,?,?,?)").bind(id).bind(state).bind(details.to_string()).bind(now).execute(&mut *tx).await.map_err(|_|Error::database())?;
        tx.commit().await.map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_recovery(&self, id: &str, recovery: &Value) -> Result<()> {
        sqlx::query("UPDATE installation_operations SET recovery_json=?,updated_at=? WHERE id=?")
            .bind(recovery.to_string())
            .bind(time())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_error(&self, id: &str, error: &Error) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::database())?;
        sqlx::query("INSERT INTO installation_operation_events(operation_id,state,details_json,created_at) SELECT id,state,?,? FROM installation_operations WHERE id=?").bind(json!({"error":error}).to_string()).bind(time()).bind(id).execute(&mut *tx).await.map_err(|_|Error::database())?;
        sqlx::query("UPDATE installation_operations SET error_json=?,updated_at=? WHERE id=?")
            .bind(json!(error).to_string())
            .bind(time())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::database())?;
        tx.commit().await.map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_records(&self, profile: &str) -> Result<Vec<Record>> {
        let values: Vec<String> =
            sqlx::query_scalar("SELECT record_json FROM managed_installations WHERE profile_id=?")
                .bind(profile)
                .fetch_all(&self.pool)
                .await
                .map_err(|_| Error::database())?;
        values
            .into_iter()
            .map(|v| {
                serde_json::from_str(&v)
                    .map_err(|_| fail("JOURNAL", "An installation record is invalid."))
            })
            .collect()
    }
    pub(crate) async fn install_reserve(&self, op: &Operation) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::database())?;
        for change in &op.plan.changes {
            let key = hash(
                change
                    .target
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_bytes(),
            );
            sqlx::query("INSERT INTO installation_target_locks(target_key,operation_id) VALUES(?,?) ON CONFLICT(target_key) DO NOTHING") .bind(&key).bind(&op.id).execute(&mut *tx).await.map_err(|_|Error::database())?;
            let owner: String = sqlx::query_scalar(
                "SELECT operation_id FROM installation_target_locks WHERE target_key=?",
            )
            .bind(&key)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| Error::database())?;
            if owner != op.id {
                return Err(fail(
                    "BUSY",
                    "Another unfinished operation owns this target.",
                ));
            }
        }
        let updated=sqlx::query("UPDATE installation_operations SET approved_at=?,state='DOWNLOADING',updated_at=? WHERE id=? AND state IN ('WAITING_FOR_APPROVAL','INTERRUPTED','FAILED')").bind(time()).bind(time()).bind(&op.id).execute(&mut *tx).await.map_err(|_|Error::database())?;
        if updated.rows_affected() != 1 {
            return Err(fail(
                "APPROVAL",
                "This operation cannot be approved in its current state.",
            ));
        }
        tx.commit().await.map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_unlock(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM installation_target_locks WHERE operation_id=?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::database())?;
        Ok(())
    }
    pub(crate) async fn install_history(&self, profile: &str) -> Result<Value> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM installation_operations WHERE profile_id=? AND (state='INTERRUPTED' OR id IN (SELECT id FROM installation_operations WHERE profile_id=? ORDER BY created_at DESC LIMIT 40)) ORDER BY created_at DESC").bind(profile).bind(profile).fetch_all(&self.pool).await.map_err(|_|Error::database())?;
        let mut result = Vec::new();
        for id in ids {
            let op = self.install_operation(&id).await?;
            let events:Vec<(String,String,i64)>=sqlx::query_as("SELECT state,details_json,created_at FROM installation_operation_events WHERE operation_id=? ORDER BY id").bind(&id).fetch_all(&self.pool).await.map_err(|_|Error::database())?;
            result.push(json!({"operation":op,"events":events.into_iter().map(|(state,details,at)|json!({"state":state,"details":serde_json::from_str::<Value>(&details).unwrap_or(Value::Null),"at":at})).collect::<Vec<_>>()}));
        }
        Ok(json!(result))
    }
    pub(crate) async fn install_interrupt(&self, profile: &str) -> Result<()> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM installation_operations WHERE profile_id=? AND state IN ('DOWNLOADING','STAGING','VALIDATING','BACKING_UP','INSTALLING','VERIFYING','ROLLING_BACK')").bind(profile).fetch_all(&self.pool).await.map_err(|_|Error::database())?;
        for id in ids {
            self.install_event(&id,"INTERRUPTED",&json!({"reason":"The process ended before a verified terminal state. No success is inferred."})).await?;
        }
        Ok(())
    }
}
