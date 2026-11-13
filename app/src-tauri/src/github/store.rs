use super::{
    http::{CacheEntry, RateState},
    types::{ApiError, Assessment, Classification, Outcome, Repository, digest, now},
};
use crate::db::Database;
use serde_json::{Value, json};

impl Database {
    pub async fn gh_cache(&self, key: &str) -> Result<Option<CacheEntry>, ApiError> {
        sqlx::query_as("SELECT etag,payload_json,fetched_at_unix_ms,last_modified,validated_at_unix_ms,expires_at_unix_ms FROM github_api_cache WHERE cache_key=?").bind(key).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())
    }
    pub async fn gh_save_cache(&self, key: &str, c: &CacheEntry) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_api_cache(cache_key,etag,payload_json,fetched_at_unix_ms,response_status,last_modified,validated_at_unix_ms,expires_at_unix_ms) VALUES(?,?,?,?,200,?,?,?) ON CONFLICT(cache_key) DO UPDATE SET etag=excluded.etag,payload_json=excluded.payload_json,fetched_at_unix_ms=excluded.fetched_at_unix_ms,last_modified=excluded.last_modified,validated_at_unix_ms=excluded.validated_at_unix_ms,expires_at_unix_ms=excluded.expires_at_unix_ms")
        .bind(key).bind(&c.etag).bind(&c.payload_json).bind(c.fetched_at_unix_ms).bind(&c.last_modified).bind(c.validated_at_unix_ms).bind(c.expires_at_unix_ms).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_rate(&self, scope: &str, resource: &str) -> Result<RateState, ApiError> {
        Ok(sqlx::query_as::<_, RateState>(
            "SELECT * FROM github_rate_state WHERE scope=? AND resource=?",
        )
        .bind(scope)
        .bind(resource)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ApiError::database())?
        .unwrap_or_else(|| RateState {
            scope: scope.into(),
            resource: resource.into(),
            ..Default::default()
        }))
    }
    pub async fn gh_save_rate(&self, r: &RateState) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_rate_state(scope,resource,remaining,limit_value,reset_at,blocked_until,last_request_at,failures) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(scope,resource) DO UPDATE SET remaining=excluded.remaining,limit_value=excluded.limit_value,reset_at=excluded.reset_at,blocked_until=excluded.blocked_until,last_request_at=excluded.last_request_at,failures=excluded.failures").bind(&r.scope).bind(&r.resource).bind(r.remaining).bind(r.limit_value).bind(r.reset_at).bind(r.blocked_until).bind(r.last_request_at).bind(r.failures).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_request_started(
        &self,
        scope: &str,
        resource: &str,
        time: i64,
    ) -> Result<(), ApiError> {
        let mut r = self.gh_rate(scope, resource).await?;
        r.last_request_at = time;
        r.remaining = r.remaining.map(|n| (n - 1).max(0));
        self.gh_save_rate(&r).await
    }
    pub async fn gh_repository_names(&self) -> Result<Vec<String>, ApiError> {
        sqlx::query_scalar("SELECT DISTINCT lower(source_owner||'/'||source_repo) FROM entities WHERE source_type='github' AND id NOT IN ('superpowers','vercel-agent-skills') ORDER BY 1").fetch_all(&self.pool).await.map_err(|_|ApiError::database())
    }
    pub async fn gh_repository(
        &self,
        repo: &Repository,
        outcome: &Outcome,
        requested: &str,
    ) -> Result<(), ApiError> {
        if outcome.warning.is_some() {
            // Retain the last confirmed availability and validation timestamp.
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(|_| ApiError::database())?;
        sqlx::query("INSERT INTO github_repositories(id,full_name,metadata_json,fetched_at,validated_at,availability) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET full_name=excluded.full_name,metadata_json=excluded.metadata_json,fetched_at=excluded.fetched_at,validated_at=excluded.validated_at,availability=excluded.availability")
            .bind(repo.id).bind(&repo.full_name).bind(serde_json::to_string(repo).unwrap()).bind(outcome.fetched_at).bind(outcome.validated_at).bind(if repo.disabled{"DISABLED"}else if repo.archived{"ARCHIVED"}else{"AVAILABLE"}).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        for alias in [requested, &repo.full_name] {
            sqlx::query("INSERT INTO github_repository_aliases(normalized_name,repository_id) VALUES(?,?) ON CONFLICT(normalized_name) DO UPDATE SET repository_id=excluded.repository_id").bind(alias).bind(repo.id).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        }
        // Bind existing immutable IDs, even after GitHub renames a repository.
        sqlx::query("INSERT INTO github_capability_sources(entity_id,repository_id,source_path,kind) SELECT e.id,?,e.source_path,e.kind FROM entities e WHERE e.source_type='github' AND lower(e.source_owner||'/'||e.source_repo) IN(lower(?),lower(?)) AND e.id NOT IN ('superpowers','vercel-agent-skills') ON CONFLICT DO NOTHING")
            .bind(repo.id).bind(requested).bind(&repo.full_name).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        tx.commit().await.map_err(|_| ApiError::database())
    }
    pub async fn gh_unavailable(&self, name: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE github_repositories SET availability='UNAVAILABLE',validated_at=? WHERE id IN(SELECT repository_id FROM github_repository_aliases WHERE normalized_name=?)").bind(now()).bind(name).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_start_run(&self, id: &str, operation: &Value) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_sync_runs(id,operation_json,status,started_at) VALUES(?,?,'RUNNING',?)").bind(id).bind(operation.to_string()).bind(now()).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_finish_run(
        &self,
        id: &str,
        status: &str,
        summary: &Value,
        retry: Option<i64>,
    ) -> Result<(), ApiError> {
        sqlx::query("UPDATE github_sync_runs SET status=?,summary_json=?,retry_at=?,finished_at=? WHERE id=?").bind(status).bind(summary.to_string()).bind(retry).bind(now()).bind(id).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_event(
        &self,
        run: &str,
        entity: Option<&str>,
        kind: &str,
        details: &Value,
    ) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_discovery_events(run_id,entity_id,event_type,details_json,created_at) VALUES(?,?,?,?,?)").bind(run).bind(entity).bind(kind).bind(details.to_string()).bind(now()).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_operation(&self, id: &str) -> Result<Value, ApiError> {
        let raw: String =
            sqlx::query_scalar("SELECT operation_json FROM github_sync_runs WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| ApiError::database())?
                .ok_or_else(|| {
                    ApiError::new(
                        "NOT_FOUND",
                        "The previous discovery operation was not found.",
                    )
                })?;
        serde_json::from_str(&raw).map_err(|_| ApiError::database())
    }
    pub async fn gh_manual_classification(
        &self,
        id: &str,
        classification: &Classification,
    ) -> Result<(), ApiError> {
        if self
            .get_capability(id)
            .await
            .map_err(ApiError::from)?
            .is_none()
        {
            return Err(ApiError::new("NOT_FOUND", "Capability was not found."));
        }
        self.gh_save_classification(id, classification).await
    }
    pub async fn gh_save_classification(
        &self,
        id: &str,
        c: &Classification,
    ) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_classifications(entity_id,primary_function,related_json,state,confidence,evidence_json,manual,checked_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(entity_id) DO UPDATE SET primary_function=excluded.primary_function,related_json=excluded.related_json,state=excluded.state,confidence=excluded.confidence,evidence_json=excluded.evidence_json,manual=excluded.manual,checked_at=excluded.checked_at WHERE github_classifications.manual=0 OR excluded.manual=1")
            .bind(id).bind(&c.primary).bind(json!(c.related).to_string()).bind(&c.state).bind(c.confidence).bind(json!(c.evidence).to_string()).bind(c.manual).bind(now()).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_save_capability(
        &self,
        repo: &Repository,
        a: &Assessment,
        commit: &str,
        run: &str,
    ) -> Result<(String, bool), ApiError> {
        let known:Option<String>=sqlx::query_scalar("SELECT entity_id FROM github_capability_sources WHERE repository_id=? AND source_path=? AND kind=?").bind(repo.id).bind(&a.path).bind(&a.kind).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())?;
        let id = known.clone().unwrap_or_else(|| {
            format!(
                "github:{}:{}:{}",
                repo.id,
                a.kind,
                &digest(a.path.as_bytes())[..24]
            )
        });
        let old:Option<String>=sqlx::query_scalar("SELECT blob_sha FROM github_capability_versions WHERE entity_id=? ORDER BY id DESC LIMIT 1").bind(&id).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())?;
        let old=match old{Some(value)=>Some(value),None=>sqlx::query_scalar::<_,Option<String>>("SELECT json_extract(metadata_json,'$.discovery.sourceBlobSha') FROM entities WHERE id=?").bind(&id).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())?.flatten()};
        let previous_fingerprint:Option<String>=sqlx::query_scalar::<_,Option<String>>("SELECT package_fingerprint FROM github_capability_versions WHERE entity_id=? ORDER BY id DESC LIMIT 1").bind(&id).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())?.flatten();
        let changed = old.as_ref().is_some_and(|sha| *sha != a.blob_sha)
            || previous_fingerprint
                .as_ref()
                .is_some_and(|fp| Some(fp) != a.package_fingerprint.as_ref());
        let prior_update: Option<String> = sqlx::query_scalar::<_,Option<String>>("SELECT json_extract(payload_json,'$.updateState') FROM github_capability_intelligence WHERE entity_id=?").bind(&id).fetch_optional(&self.pool).await.map_err(|_|ApiError::database())?.flatten();
        let mut intelligence = a.intelligence(commit, now());
        intelligence["sourceState"] = json!("PRESENT");
        intelligence["updateState"] = json!(if changed
            || prior_update.as_deref() == Some("UPDATE_AVAILABLE")
        {
            "UPDATE_AVAILABLE"
        } else if old.is_none() {
            "VERSION_OBSERVED"
        } else {
            "UP_TO_DATE"
        });
        let functions: Vec<String> = a
            .classification
            .primary
            .iter()
            .chain(a.classification.related.iter())
            .cloned()
            .collect();
        let metadata = json!({"discovery":{"id":id,"kind":a.kind,"name":a.name,"summary":a.description,"owner":repo.full_name.split('/').next().unwrap(),"repo":repo.full_name.split('/').nth(1).unwrap(),"githubUrl":format!("https://github.com/{}/blob/{}/{}",repo.full_name,commit,a.path),"category":"Unclassified","functionCategories":if functions.is_empty(){vec!["unclassified".to_string()]}else{functions},"capabilities":[],"license":repo.license.as_deref().unwrap_or("License not verified"),"mark":a.kind,"sourcePath":a.path,"sourceRef":commit,"sourceBlobSha":a.blob_sha,"repositoryStars":repo.stars,"repositoryPushedAt":repo.pushed_at,"verifiedAt":now(),"setupRequirements":a.requirements,"compatibility":a.compatibility,"compatibilityEvidence":format!("https://github.com/{}/blob/{}/{}",repo.full_name,commit,a.path)}});
        let mut tx = self.pool.begin().await.map_err(|_| ApiError::database())?;
        // Existing curated entries and their stable IDs/metadata remain intact.
        sqlx::query("INSERT INTO entities(id,kind,name,summary,source_type,source_owner,source_repo,source_ref,source_url,license_spdx,metadata_json,source_path) VALUES(?,?,?,?,'github',?,?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING")
            .bind(&id).bind(if a.kind=="repository"{"capability"}else{&a.kind}).bind(&a.name).bind(&a.description).bind(repo.full_name.split('/').next()).bind(repo.full_name.split('/').nth(1)).bind(commit).bind(format!("https://github.com/{}/blob/{}/{}",repo.full_name,commit,a.path)).bind(&repo.license).bind(metadata.to_string()).bind(&a.path).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        sqlx::query("INSERT INTO github_capability_sources(entity_id,repository_id,source_path,kind) VALUES(?,?,?,?) ON CONFLICT DO NOTHING").bind(&id).bind(repo.id).bind(&a.path).bind(&a.kind).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        sqlx::query("INSERT INTO github_capability_versions(entity_id,commit_sha,blob_sha,package_fingerprint,manifest_version,metadata_json,observed_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(entity_id,commit_sha,blob_sha) DO NOTHING")
            .bind(&id).bind(commit).bind(&a.blob_sha).bind(&a.package_fingerprint).bind(&a.manifest_version).bind(intelligence.to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        sqlx::query("INSERT INTO github_capability_intelligence(entity_id,payload_json,checked_at) VALUES(?,?,?) ON CONFLICT(entity_id) DO UPDATE SET payload_json=excluded.payload_json,checked_at=excluded.checked_at")
            .bind(&id).bind(intelligence.to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        if let Some(fingerprint) = a.package_fingerprint.as_ref() {
            let duplicates:Vec<String>=sqlx::query_scalar("SELECT DISTINCT entity_id FROM github_capability_versions WHERE package_fingerprint=? AND entity_id!=?").bind(fingerprint).bind(&id).fetch_all(&mut *tx).await.map_err(|_|ApiError::database())?;
            for other in duplicates {
                let (left, right) = if other < id {
                    (other.as_str(), id.as_str())
                } else {
                    (id.as_str(), other.as_str())
                };
                sqlx::query("INSERT INTO github_duplicate_links(left_entity_id,right_entity_id,reason,checked_at) VALUES(?,?,'EXACT_PACKAGE_FINGERPRINT',?) ON CONFLICT DO NOTHING").bind(left).bind(right).bind(now()).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
            }
        }
        sqlx::query("INSERT INTO github_discovery_events(run_id,entity_id,event_type,details_json,created_at) VALUES(?,?,?,?,?)").bind(run).bind(&id).bind(if known.is_none(){"DISCOVERED"}else if changed{"UPDATE_AVAILABLE"}else{"INSPECTED"}).bind(json!({"commit":commit,"blob":a.blob_sha,"compatibility":a.compatibility}).to_string()).bind(now()).execute(&mut *tx).await.map_err(|_|ApiError::database())?;
        tx.commit().await.map_err(|_| ApiError::database())?;
        self.gh_save_classification(&id, &a.classification).await?;
        Ok((id, known.is_none()))
    }
    pub async fn gh_source_state(
        &self,
        id: &str,
        state: &str,
        commit: &str,
        run: &str,
    ) -> Result<(), ApiError> {
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT payload_json FROM github_capability_intelligence WHERE entity_id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ApiError::database())?;
        let mut data: Value = previous
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({"compatibility":"UNVERIFIED","reviewState":"NEEDS_REVIEW"}));
        data["sourceState"] = json!(state);
        data["sourceObservedAt"] = json!(now());
        data["observedCommit"] = json!(commit);
        if state == "REVIEW_REQUIRED" {
            data["updateState"] = json!("UPDATE_AVAILABLE");
        }
        sqlx::query("INSERT INTO github_capability_intelligence(entity_id,payload_json,checked_at) VALUES(?,?,?) ON CONFLICT(entity_id) DO UPDATE SET payload_json=excluded.payload_json,checked_at=excluded.checked_at").bind(id).bind(data.to_string()).bind(now()).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        self.gh_event(run, Some(id), state, &json!({"commit":commit}))
            .await
    }
    pub async fn gh_link_local(
        &self,
        local: &str,
        remote: &str,
        fingerprint: &str,
    ) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO github_local_links(local_entity_id,remote_entity_id,evidence_json,checked_at) VALUES(?,?,?,?) ON CONFLICT(local_entity_id,remote_entity_id) DO UPDATE SET evidence_json=excluded.evidence_json,checked_at=excluded.checked_at").bind(local).bind(remote).bind(json!({"method":"EXACT_PACKAGE_FINGERPRINT","fingerprint":fingerprint,"installationRecordUnchanged":true}).to_string()).bind(now()).execute(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(())
    }
    pub async fn gh_state(&self) -> Result<Value, ApiError> {
        let repos:Vec<(i64,String,String,i64,i64,String)>=sqlx::query_as("SELECT id,full_name,metadata_json,fetched_at,validated_at,availability FROM github_repositories ORDER BY full_name").fetch_all(&self.pool).await.map_err(|_|ApiError::database())?;
        let assessments: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT entity_id,payload_json,checked_at FROM github_capability_intelligence",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| ApiError::database())?;
        let classifications:Vec<(String,Option<String>,String,String,f64,String,bool)>=sqlx::query_as("SELECT entity_id,primary_function,related_json,state,confidence,evidence_json,manual FROM github_classifications").fetch_all(&self.pool).await.map_err(|_|ApiError::database())?;
        let rates: Vec<RateState> = sqlx::query_as("SELECT * FROM github_rate_state")
            .fetch_all(&self.pool)
            .await
            .map_err(|_| ApiError::database())?;
        let history:Vec<(String,String,String,i64,Option<i64>,Option<i64>,String)>=sqlx::query_as("SELECT id,operation_json,status,started_at,finished_at,retry_at,summary_json FROM github_sync_runs ORDER BY started_at DESC LIMIT 30").fetch_all(&self.pool).await.map_err(|_|ApiError::database())?;
        let aliases: Vec<(String, i64)> =
            sqlx::query_as("SELECT normalized_name,repository_id FROM github_repository_aliases")
                .fetch_all(&self.pool)
                .await
                .map_err(|_| ApiError::database())?;
        let links:Vec<(String,String,String,i64)>=sqlx::query_as("SELECT local_entity_id,remote_entity_id,evidence_json,checked_at FROM github_local_links").fetch_all(&self.pool).await.map_err(|_|ApiError::database())?;
        Ok(
            json!({"repositories":repos.into_iter().map(|(id,name,meta,fetched,validated,availability)|json!({"id":id,"fullName":name,"aliases":aliases.iter().filter(|(_,repo)|*repo==id).map(|(name,_)|name).collect::<Vec<_>>(),"metadata":serde_json::from_str::<Value>(&meta).unwrap_or(Value::Null),"fetchedAt":fetched,"validatedAt":validated,"availability":availability})).collect::<Vec<_>>(),"assessments":assessments.into_iter().map(|(id,payload,checked)|json!({"entityId":id,"data":serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),"checkedAt":checked})).collect::<Vec<_>>(),"classifications":classifications.into_iter().map(|(id,primary,related,state,confidence,evidence,manual)|json!({"entityId":id,"primary":primary,"related":serde_json::from_str::<Value>(&related).unwrap_or(json!([])),"state":state,"confidence":confidence,"evidence":serde_json::from_str::<Value>(&evidence).unwrap_or(json!([])),"manual":manual})).collect::<Vec<_>>(),"rateLimits":rates,"history":history.into_iter().map(|(id,op,status,started,finished,retry,summary)|json!({"id":id,"operation":serde_json::from_str::<Value>(&op).unwrap_or(Value::Null),"status":status,"startedAt":started,"finishedAt":finished,"retryAt":retry,"summary":serde_json::from_str::<Value>(&summary).unwrap_or(Value::Null)})).collect::<Vec<_>>(),"localLinks":links.into_iter().map(|(local,remote,evidence,checked)|json!({"localEntityId":local,"remoteEntityId":remote,"evidence":serde_json::from_str::<Value>(&evidence).unwrap_or(Value::Null),"checkedAt":checked})).collect::<Vec<_>>() }),
        )
    }
}
