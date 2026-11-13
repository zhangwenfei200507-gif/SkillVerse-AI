pub(crate) mod analysis;
pub(crate) mod credentials;
pub(crate) mod http;
pub(crate) mod planner;
mod store;
#[cfg(test)]
mod tests;
pub mod types;

use crate::db::Database;
use base64::Engine;
use credentials::SystemCredentials;
use http::{GitHubHttp, RequestContext};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, atomic::Ordering},
};
use types::{
    ApiError, Classification, Operation, Outcome, Repository, normalize_repository, safe_path,
};

pub struct GitHubDiscoveryService {
    http: GitHubHttp,
    jobs: Mutex<HashMap<String, RequestContext>>,
    provider: Box<dyn analysis::AIClassificationProvider>,
}
impl GitHubDiscoveryService {
    pub async fn new(database: Database) -> Result<Self, ApiError> {
        sqlx::query("UPDATE github_sync_runs SET status='INTERRUPTED',finished_at=?,summary_json=json_set(summary_json,'$.error',json(?)) WHERE status='RUNNING'").bind(types::now()).bind(json!({"kind":"INTERRUPTED","message":"The previous process ended before completion. Saved progress is retained; resume explicitly."}).to_string()).execute(&database.pool).await.map_err(|_|ApiError::database())?;
        Ok(Self {
            http: GitHubHttp::new(database, Arc::new(SystemCredentials::default()))?,
            jobs: Mutex::new(HashMap::new()),
            provider: Box::new(analysis::NoAIProvider),
        })
    }
    pub async fn state(&self) -> Result<Value, ApiError> {
        let mut state = self.http.db.gh_state().await?;
        state["tokenConfigured"] = json!(self.http.credentials.read()?.is_some());
        state["classificationProvider"] = json!("rules; AI provider not configured");
        Ok(state)
    }
    pub fn cancel(&self, id: &str) -> Result<(), ApiError> {
        if let Some(context) = self
            .jobs
            .lock()
            .map_err(|_| ApiError::new("BUSY", "Operation state is unavailable."))?
            .get(id)
        {
            context.cancel.cancel();
        }
        Ok(())
    }
    pub async fn set_token(&self, token: Option<String>) -> Result<(), ApiError> {
        if !self
            .jobs
            .lock()
            .map_err(|_| ApiError::database())?
            .is_empty()
        {
            return Err(ApiError::new(
                "BUSY",
                "Finish or cancel the current GitHub operation before changing credentials.",
            ));
        }
        match token {
            Some(value) => self.http.credentials.save(&value)?,
            None => self.http.credentials.remove()?,
        };
        // Account changes cannot reuse the previous account's HTTP/rate partition.
        sqlx::query("DELETE FROM github_rate_state WHERE scope='public-authenticated'")
            .execute(&self.http.db.pool)
            .await
            .map_err(|_| ApiError::database())?;
        // Cache keys are opaque hashes, so invalidate the HTTP cache only. This
        // does not remove any repository, capability, version, or scan record.
        sqlx::query("UPDATE github_api_cache SET expires_at_unix_ms=0")
            .execute(&self.http.db.pool)
            .await
            .map_err(|_| ApiError::database())?;
        Ok(())
    }
    pub async fn classify_manually(
        &self,
        id: &str,
        primary: Option<String>,
        related: Vec<String>,
    ) -> Result<(), ApiError> {
        let valid = analysis::category_keys();
        if primary
            .iter()
            .chain(related.iter())
            .any(|key| !valid.contains(key))
            || related.len() > 5
        {
            return Err(ApiError::new(
                "CATEGORY",
                "Choose valid functional categories.",
            ));
        }
        let mut related = related;
        related.sort();
        related.dedup();
        related.retain(|key| Some(key) != primary.as_ref());
        let c=Classification{state:if primary.is_some(){"CLASSIFIED"}else{"PENDING_REVIEW"}.into(),primary,related,confidence:1.0,evidence:vec!["Classification set explicitly by the user; installation evidence was not modified.".into()],manual:true};
        self.http.db.gh_manual_classification(id, &c).await
    }
    pub async fn resume(&self, run: &str, new_id: &str) -> Result<Value, ApiError> {
        let op = self.http.db.gh_operation(run).await?;
        let mut op: Operation = serde_json::from_value(op)
            .map_err(|_| ApiError::new("OPERATION", "Saved operation is invalid."))?;
        op.force = false;
        self.execute(new_id, op).await
    }
    pub async fn execute(&self, id: &str, operation: Operation) -> Result<Value, ApiError> {
        if id.is_empty()
            || id.len() > 80
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        {
            return Err(ApiError::new("OPERATION", "Invalid operation identifier."));
        }
        if !["search", "inspect", "refresh"].contains(&operation.action.as_str()) {
            return Err(ApiError::new(
                "OPERATION",
                "Unsupported discovery operation.",
            ));
        }
        if let Some(query) = operation.query.as_ref() {
            if query.len() > 180
                || query.contains(['\r', '\n'])
                || ["ghp_", "github_pat_", "is:private", "visibility:private"]
                    .iter()
                    .any(|s| query.to_lowercase().contains(s))
            {
                return Err(ApiError::new(
                    "INVALID_QUERY",
                    "Use a bounded public-repository search without credentials.",
                ));
            }
        }
        if let Some(repo) = operation.repository.as_ref() {
            normalize_repository(repo)?;
        }
        let context = RequestContext::new();
        {
            let mut jobs = self.jobs.lock().map_err(|_| ApiError::database())?;
            if !jobs.is_empty() {
                return Err(ApiError::new(
                    "BUSY",
                    "Another GitHub operation is running. Cancel it before starting another.",
                ));
            }
            jobs.insert(id.into(), context.clone());
        }
        if let Err(error) = self.http.db.gh_start_run(id, &json!(operation)).await {
            self.jobs.lock().unwrap().remove(id);
            return Err(error);
        }
        let result = match operation.action.as_str() {
            "search" => self.search_repositories(&operation, &context).await,
            "inspect" => self.inspect(&operation, id, &context).await,
            _ => {
                self.refresh_repository_metadata(&operation, id, &context)
                    .await
            }
        };
        self.jobs
            .lock()
            .map_err(|_| ApiError::database())?
            .remove(id);
        let (status, retry) = match result.as_ref() {
            Ok(value) if value["warning"]["kind"] == "RATE_LIMITED" => {
                ("DEFERRED", value["warning"]["retryAt"].as_i64())
            }
            Ok(value) if value["warning"].is_object() => ("PARTIAL", None),
            Ok(_) => ("COMPLETE", None),
            Err(error) if error.kind == "RATE_LIMITED" => ("DEFERRED", error.retry_at),
            Err(error) if error.kind == "CANCELLED" => ("CANCELLED", None),
            Err(_) => ("FAILED", None),
        };
        let summary = match result.as_ref() {
            Ok(value) => json!({"requests":context.requests.load(Ordering::SeqCst),"result":value}),
            Err(error) => json!({"requests":context.requests.load(Ordering::SeqCst),"error":error}),
        };
        self.http
            .db
            .gh_finish_run(id, status, &summary, retry)
            .await?;
        self.http
            .db
            .gh_event(
                id,
                None,
                status,
                &json!({"requests":context.requests.load(Ordering::SeqCst)}),
            )
            .await?;
        result
    }
    pub async fn search_repositories(
        &self,
        op: &Operation,
        ctx: &RequestContext,
    ) -> Result<Value, ApiError> {
        let query = op
            .query
            .as_deref()
            .filter(|q| !q.trim().is_empty())
            .ok_or_else(|| ApiError::new("INVALID_QUERY", "Enter a repository search."))?;
        let page = op.page.unwrap_or(1);
        if page == 0 || page > 200 {
            return Err(ApiError::new(
                "INVALID_QUERY",
                "Search page is outside GitHub's 1,000-result window.",
            ));
        }
        let q = format!("{} is:public", query.trim());
        let page_string = page.to_string();
        let outcome = self
            .http
            .get(
                &endpoint(
                    "/search/repositories",
                    &[("q", &q), ("per_page", "5"), ("page", &page_string)],
                ),
                300_000,
                op.force,
                ctx,
            )
            .await?;
        let repositories = outcome.body["items"]
            .as_array()
            .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "Search results are invalid."))?
            .iter()
            .filter_map(|item| Repository::parse(item).ok())
            .collect::<Vec<_>>();
        // Search results are leads only. Import requires manifest inspection.
        Ok(
            json!({"repositories":repositories,"totalCount":outcome.body["total_count"],"incompleteResults":outcome.body["incomplete_results"],"page":page,"hasNext":(page as i64)*5<outcome.body["total_count"].as_i64().unwrap_or(0).min(1000),"source":outcome.source,"fetchedAt":outcome.fetched_at,"validatedAt":outcome.validated_at,"warning":outcome.warning}),
        )
    }
    pub async fn get_repository_details(
        &self,
        name: &str,
        force: bool,
        ctx: &RequestContext,
    ) -> Result<(Repository, Outcome), ApiError> {
        let name = normalize_repository(name)?;
        let outcome = match self
            .http
            .get(&format!("/repos/{name}"), 6 * 3_600_000, force, ctx)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                if error.kind == "NOT_FOUND" {
                    self.http.db.gh_unavailable(&name).await?;
                }
                return Err(error);
            }
        };
        let repo = Repository::parse(&outcome.body)?;
        self.http.db.gh_repository(&repo, &outcome, &name).await?;
        Ok((repo, outcome))
    }
    pub async fn get_repository_activity(
        &self,
        repo: &Repository,
        force: bool,
        ctx: &RequestContext,
    ) -> Result<Outcome, ApiError> {
        self.http
            .get(
                &endpoint(
                    &format!("/repos/{}/commits", repo.full_name),
                    &[("sha", &repo.default_branch), ("per_page", "5")],
                ),
                900_000,
                force,
                ctx,
            )
            .await
    }
    pub async fn get_repository_release_info(
        &self,
        repo: &Repository,
        force: bool,
        ctx: &RequestContext,
    ) -> Result<Value, ApiError> {
        match self
            .http
            .get(
                &format!("/repos/{}/releases/latest", repo.full_name),
                3_600_000,
                force,
                ctx,
            )
            .await
        {
            Ok(result) => {
                if let Some(error) = result.warning {
                    return Err(error);
                }
                Ok(result.body)
            }
            Err(error) if error.kind == "NOT_FOUND" => Ok(Value::Null),
            Err(error) => Err(error),
        }
    }
    pub async fn get_repository_contents(
        &self,
        repo: &Repository,
        path: &str,
        revision: &str,
        ctx: &RequestContext,
    ) -> Result<(String, String), ApiError> {
        let path = safe_path(path)?;
        let result = self
            .http
            .get(
                &endpoint(
                    &format!("/repos/{}/contents/{path}", repo.full_name),
                    &[("ref", revision)],
                ),
                365 * 86_400_000,
                false,
                ctx,
            )
            .await?;
        if let Some(error) = result.warning {
            return Err(error);
        }
        if result.body["type"] != "file"
            || result.body["encoding"] != "base64"
            || result.body["path"].as_str() != Some(&path)
        {
            return Err(ApiError::new(
                "UNSAFE_FILE",
                "Manifest is not an ordinary file at the expected path. Symlinks are not followed.",
            ));
        }
        let raw = result.body["content"]
            .as_str()
            .ok_or_else(|| {
                ApiError::new("CONTENTS", "GitHub did not provide bounded file contents.")
            })?
            .replace(['\n', '\r'], "");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|_| ApiError::new("CONTENTS", "GitHub content encoding was invalid."))?;
        if bytes.len() > 256 * 1024 {
            return Err(ApiError::new(
                "CONTENTS",
                "Source file exceeds the read-only inspection limit.",
            ));
        }
        let actual = git_blob(&bytes);
        if result.body["sha"].as_str() != Some(actual.as_str()) {
            return Err(ApiError::new(
                "CONTENTS",
                "Source bytes did not match their GitHub blob identity.",
            ));
        }
        Ok((
            String::from_utf8(bytes)
                .map_err(|_| ApiError::new("CONTENTS", "Manifest is not UTF-8 text."))?,
            actual,
        ))
    }
    pub async fn get_repository_manifest(
        &self,
        repo: &Repository,
        path: &str,
        revision: &str,
        ctx: &RequestContext,
    ) -> Result<(String, String), ApiError> {
        self.get_repository_contents(repo, path, revision, ctx)
            .await
    }
    async fn tree(
        &self,
        repo: &Repository,
        commit: &str,
        ctx: &RequestContext,
    ) -> Result<BTreeMap<String, (String, String)>, ApiError> {
        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApiError::new(
                "REVISION",
                "Inspection requires an immutable GitHub commit SHA.",
            ));
        }
        let outcome = self
            .http
            .get(
                &format!("/repos/{}/git/trees/{commit}?recursive=1", repo.full_name),
                365 * 86_400_000,
                false,
                ctx,
            )
            .await?;
        if let Some(error) = outcome.warning {
            return Err(error);
        }
        if outcome.body["truncated"] == true {
            return Err(ApiError::new(
                "INCOMPLETE_TREE",
                "GitHub returned a truncated tree; compatibility was not asserted.",
            ));
        }
        Ok(outcome.body["tree"]
            .as_array()
            .ok_or_else(|| ApiError::new("TREE", "Repository tree was invalid."))?
            .iter()
            .filter(|item| item["type"] == "blob" || item["type"] == "commit")
            .filter_map(|item| {
                Some((
                    item["path"].as_str()?.to_string(),
                    (
                        item["sha"].as_str()?.to_string(),
                        item["mode"].as_str()?.to_string(),
                    ),
                ))
            })
            .collect())
    }
    async fn scoped_tree(
        &self,
        repo: &Repository,
        commit: &str,
        root: &str,
        ctx: &RequestContext,
    ) -> Result<BTreeMap<String, (String, String)>, ApiError> {
        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApiError::new(
                "REVISION",
                "Inspection requires an immutable commit identity.",
            ));
        }
        if root.is_empty() {
            return self.tree(repo, commit, ctx).await;
        }
        let root = safe_path(root)?;
        let mut revision = commit.to_string();
        let mut prefix = String::new();
        let mut readme = None;
        for segment in root.split('/') {
            let response = self
                .http
                .get(
                    &format!("/repos/{}/git/trees/{revision}", repo.full_name),
                    365 * 86_400_000,
                    false,
                    ctx,
                )
                .await?;
            if let Some(error) = response.warning {
                return Err(error);
            }
            let entries = response.body["tree"]
                .as_array()
                .ok_or_else(|| ApiError::new("TREE", "Repository subtree is invalid."))?;
            if prefix.is_empty() {
                readme = entries
                    .iter()
                    .find(|v| {
                        v["type"] == "blob"
                            && v["path"]
                                .as_str()
                                .is_some_and(|p| p.to_lowercase().starts_with("readme"))
                    })
                    .cloned();
            }
            let entry = entries
                .iter()
                .find(|v| v["path"] == segment)
                .ok_or_else(|| {
                    ApiError::new(
                        "SOURCE_MISSING",
                        "The package directory is missing; saved candidates were retained.",
                    )
                })?;
            if entry["type"] != "tree" || entry["mode"] != "040000" {
                return Err(ApiError::new(
                    "UNSAFE_PATH",
                    "The package directory is a symlink or unsupported tree.",
                ));
            }
            revision = entry["sha"]
                .as_str()
                .filter(|sha| sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
                .ok_or_else(|| ApiError::new("TREE", "Invalid immutable subtree identity."))?
                .to_string();
            prefix = if prefix.is_empty() {
                segment.to_string()
            } else {
                format!("{prefix}/{segment}")
            };
        }
        let subtree = self.tree(repo, &revision, ctx).await?;
        let mut result = subtree
            .into_iter()
            .map(|(path, entry)| (format!("{prefix}/{path}"), entry))
            .collect::<BTreeMap<_, _>>();
        if let Some(readme) = readme {
            if let (Some(path), Some(sha), Some(mode)) = (
                readme["path"].as_str(),
                readme["sha"].as_str(),
                readme["mode"].as_str(),
            ) {
                result.insert(path.into(), (sha.into(), mode.into()));
            }
        }
        Ok(result)
    }
    async fn inspect(
        &self,
        op: &Operation,
        run: &str,
        ctx: &RequestContext,
    ) -> Result<Value, ApiError> {
        let name = op
            .repository
            .as_deref()
            .ok_or_else(|| ApiError::new("REPOSITORY", "Choose a repository to inspect."))?;
        let (repo, source) = self.get_repository_details(name, op.force, ctx).await?;
        if let Some(error) = source.warning {
            return Err(error);
        }
        if repo.disabled {
            return Err(ApiError::new(
                "UNAVAILABLE",
                "Repository is disabled; existing capabilities were retained.",
            ));
        }
        let activity = self.get_repository_activity(&repo, op.force, ctx).await?;
        if let Some(error) = activity.warning {
            return Err(error);
        }
        let commit = activity.body[0]["sha"]
            .as_str()
            .ok_or_else(|| ApiError::new("REVISION", "No repository commit is available."))?
            .to_string();
        let focused = if let Some(id) = op.entity_id.as_ref() {
            let item = self
                .http
                .db
                .get_capability(id)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::new("NOT_FOUND", "Capability was not found."))?;
            let bound: Option<i64> = sqlx::query_scalar(
                "SELECT repository_id FROM github_capability_sources WHERE entity_id=?",
            )
            .bind(id)
            .fetch_optional(&self.http.db.pool)
            .await
            .map_err(|_| ApiError::database())?;
            if bound != Some(repo.id) {
                return Err(ApiError::new(
                    "IDENTITY",
                    "The selected capability belongs to another repository.",
                ));
            }
            let metadata: Value = serde_json::from_str(&item.metadata_json).unwrap_or(Value::Null);
            metadata["discovery"]["sourcePath"]
                .as_str()
                .map(|path| (path.to_string(), item.kind))
        } else {
            None
        };
        let scope = focused
            .as_ref()
            .map(|(path, kind)| analysis::package_root(path, kind))
            .unwrap_or("");
        let tree = match self.scoped_tree(&repo, &commit, scope, ctx).await {
            Ok(tree) => tree,
            Err(error) => {
                if ["SOURCE_MISSING", "UNSAFE_PATH"].contains(&error.kind.as_str()) {
                    if let Some(id) = op.entity_id.as_deref() {
                        self.http
                            .db
                            .gh_source_state(id, &error.kind, &commit, run)
                            .await?;
                    }
                }
                return Err(error);
            }
        };
        // Compare all known components against the new immutable tree, without
        // downloading each manifest. A changed source must be inspected again.
        let known: Vec<(String,String,Option<String>,Option<String>,Option<String>)> = sqlx::query_as("SELECT s.entity_id,s.source_path,json_extract(i.payload_json,'$.latestBlob'),json_extract(i.payload_json,'$.packageRoot'),json_extract(i.payload_json,'$.packageFingerprint') FROM github_capability_sources s LEFT JOIN github_capability_intelligence i ON i.entity_id=s.entity_id WHERE s.repository_id=?").bind(repo.id).fetch_all(&self.http.db.pool).await.map_err(|_|ApiError::database())?;
        for (id, path, blob, root, fingerprint) in known {
            if !scope.is_empty() && !path.starts_with(&format!("{scope}/")) {
                continue;
            }
            let state = match tree.get(&path) {
                None => Some("SOURCE_MISSING"),
                Some((_, mode)) if mode == "120000" || mode == "160000" => Some("UNSAFE_SOURCE"),
                Some((sha, _))
                    if blob.as_ref().is_some_and(|old| old != sha)
                        || fingerprint.as_ref().is_some_and(|old| {
                            Some(old)
                                != analysis::package_fingerprint(
                                    root.as_deref().unwrap_or(""),
                                    &tree,
                                )
                                .as_ref()
                        }) =>
                {
                    Some("REVIEW_REQUIRED")
                }
                _ => None,
            };
            if let Some(state) = state {
                self.http
                    .db
                    .gh_source_state(&id, state, &commit, run)
                    .await?;
            }
        }
        let release = self
            .get_repository_release_info(&repo, op.force, ctx)
            .await?;
        let commit_detail = self
            .http
            .get(
                &format!("/repos/{}/commits/{commit}", repo.full_name),
                365 * 86_400_000,
                false,
                ctx,
            )
            .await?;
        if let Some(error) = commit_detail.warning {
            return Err(error);
        }
        let readme_path = tree
            .keys()
            .find(|path| {
                !path.contains('/')
                    && path.to_lowercase().starts_with("readme")
                    && tree
                        .get(*path)
                        .is_some_and(|(_, mode)| mode == "100644" || mode == "100755")
            })
            .cloned();
        let readme = match readme_path {
            Some(path) => match self
                .get_repository_contents(&repo, &path, &commit, ctx)
                .await
            {
                Ok((text, _)) => text,
                Err(error) if ["RATE_LIMITED", "CANCELLED"].contains(&error.kind.as_str()) => {
                    return Err(error);
                }
                Err(_) => String::new(),
            },
            None => String::new(),
        };
        let mut paths = tree
            .keys()
            .filter(|path| {
                path.ends_with("/SKILL.md")
                    || *path == "SKILL.md"
                    || path.ends_with("/plugin.json")
                    || *path == "plugin.json"
                    || path.ends_with("/.mcp.json")
                    || path.ends_with("/mcp.json")
                    || *path == ".mcp.json"
                    || *path == "mcp.json"
            })
            .cloned()
            .collect::<Vec<_>>();
        // Avoid listing both a portable and compatibility manifest as two plugins.
        paths.retain(|path| {
            !path.contains(".claude-plugin/")
                || !tree.contains_key(&path.replace(".claude-plugin/", ".codex-plugin/"))
        });
        paths.retain(|path| {
            path.contains(".codex-plugin/")
                || path.contains(".claude-plugin/")
                || !path.ends_with("plugin.json")
                || !tree.contains_key(&format!(
                    "{}.codex-plugin/plugin.json",
                    path.strip_suffix("plugin.json").unwrap()
                ))
        });
        let mut selected = None;
        if let Some(id) = op.entity_id.as_ref() {
            let item = self
                .http
                .db
                .get_capability(id)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::new("NOT_FOUND", "Capability was not found."))?;
            let metadata: Value = serde_json::from_str(&item.metadata_json).unwrap_or(Value::Null);
            selected = metadata["discovery"]["sourcePath"]
                .as_str()
                .map(str::to_string);
            if let Some(path) = selected.as_ref() {
                if tree.contains_key(path) {
                    paths.retain(|p| p != path);
                    paths.insert(0, path.clone());
                } else {
                    self.http
                        .db
                        .gh_event(
                            run,
                            Some(id),
                            "SOURCE_MISSING",
                            &json!({"path":path,"commit":commit}),
                        )
                        .await?;
                    return Err(ApiError::new(
                        "SOURCE_MISSING",
                        "The recorded component path is missing at the current revision. Its existing record was retained.",
                    ));
                }
            }
        }
        let total = paths.len();
        let page = op.page.unwrap_or(1);
        if page == 0 || page > 1000 {
            return Err(ApiError::new("PAGE", "Invalid manifest page."));
        }
        let offset = if selected.is_some() {
            0
        } else {
            (page as usize - 1) * 8
        };
        paths = paths.into_iter().skip(offset).collect();
        let limit = if selected.is_some() { 1 } else { 8 };
        paths.truncate(limit);
        let local_inventory = planner::local_inventory(&self.http.db).await?;
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            if ctx.cancel.is_cancelled() {
                return Err(ApiError::new(
                    "CANCELLED",
                    "Inspection cancelled; previously saved candidates were retained.",
                ));
            }
            if tree
                .get(&path)
                .is_some_and(|(_, mode)| mode == "120000" || mode == "160000")
            {
                failures.push(json!({"path":path,"error":"Symlink/submodule was not followed"}));
                continue;
            }
            let (text, sha) = match self
                .get_repository_manifest(&repo, &path, &commit, ctx)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    if error.kind == "RATE_LIMITED" || error.kind == "CANCELLED" {
                        return Err(error);
                    }
                    failures.push(json!({"path":path,"error":error}));
                    continue;
                }
            };
            if tree.get(&path).is_none_or(|(expected, _)| expected != &sha) {
                failures.push(
                    json!({"path":path,"error":"Commit tree and source blob identity disagree"}),
                );
                continue;
            }
            let mut assessment = analysis::assess(&path, &text, &sha, &tree, &readme, &repo);
            // Read only a bounded set of dependency/script sources. Never execute.
            let mut review_paths = assessment
                .dependencies
                .iter()
                .filter(|path| !path.starts_with("command:"))
                .cloned()
                .collect::<Vec<_>>();
            review_paths.extend(
                assessment
                    .resource_paths
                    .iter()
                    .filter(|p| p.ends_with("mcp.json") || p.ends_with(".app.json"))
                    .cloned(),
            );
            review_paths.sort();
            review_paths.dedup();
            for dependency in review_paths.iter().take(2) {
                if tree
                    .get(dependency)
                    .is_none_or(|(_, mode)| mode == "120000" || mode == "160000")
                {
                    assessment.risks.push("A declared dependency is missing, a symlink, or a submodule; it was not followed.".into());
                    continue;
                }
                match self
                    .get_repository_contents(&repo, dependency, &commit, ctx)
                    .await
                {
                    Ok((source, _)) => assessment
                        .risks
                        .extend(analysis::risks(&source, &[dependency.clone()])),
                    Err(error) if ["RATE_LIMITED", "CANCELLED"].contains(&error.kind.as_str()) => {
                        return Err(error);
                    }
                    Err(_) => assessment
                        .risks
                        .push("A declared script/dependency source could not be reviewed.".into()),
                }
            }
            assessment.risks.sort();
            assessment.risks.dedup();
            if !assessment.risks.is_empty() {
                assessment.review_state = "NEEDS_REVIEW".into();
                if assessment.compatibility == "VERIFIED_COMPATIBLE" {
                    assessment.compatibility = "LIKELY_COMPATIBLE".into();
                }
            }
            if assessment.classification.primary.is_none() {
                let previous: Option<String>=sqlx::query_scalar("SELECT e.metadata_json FROM github_capability_sources s JOIN entities e ON e.id=s.entity_id WHERE s.repository_id=? AND s.source_path=? AND s.kind=?").bind(repo.id).bind(&assessment.path).bind(&assessment.kind).fetch_optional(&self.http.db.pool).await.map_err(|_|ApiError::database())?;
                if let Some(metadata) =
                    previous.and_then(|s| serde_json::from_str::<Value>(&s).ok())
                {
                    analysis::retain_reviewed_categories(&mut assessment.classification, &metadata);
                }
            }
            if let Some(classification) = self
                .provider
                .classify(&assessment.name, &assessment.description)
            {
                assessment.classification = classification;
            }
            assessment.quality = analysis::quality(
                &repo,
                &assessment,
                &activity.body,
                &release,
                !readme.is_empty(),
            );
            let files = commit_detail.body["files"].as_array();
            let substantive = files.map(|files| {
                files
                    .iter()
                    .filter_map(|f| f["filename"].as_str())
                    .filter(|name| {
                        !name.ends_with(".md")
                            && !name.ends_with(".lock")
                            && !name.starts_with(".github/")
                    })
                    .count()
            });
            assessment.quality["latestCommitChangedFileCount"] = json!(files.map(Vec::len));
            assessment.quality["latestCommitNonDocumentationFileCount"] = json!(substantive);
            assessment.quality["maintenanceEvidenceScope"] = json!(
                "One immutable latest-commit diff sample; metadata-only and documentation-only changes are not treated as proof of feature maintenance."
            );
            let (id, new) = self
                .http
                .db
                .gh_save_capability(&repo, &assessment, &commit, run)
                .await?;
            planner::link_fingerprints(&self.http.db, &id, &assessment, &local_inventory).await?;
            results.push(json!({"entityId":id,"new":new,"name":assessment.name,"kind":assessment.kind,"compatibility":assessment.compatibility,"reviewState":assessment.review_state,"classification":assessment.classification}));
            sqlx::query("UPDATE github_sync_runs SET summary_json=? WHERE id=?").bind(json!({"requests":ctx.requests.load(Ordering::SeqCst),"completedComponents":results.len(),"manifestCount":total,"currentManifest":path}).to_string()).bind(run).execute(&self.http.db.pool).await.map_err(|_|ApiError::database())?;
        }
        Ok(
            json!({"repository":repo,"commit":commit,"components":results,"availableManifestCount":total,"inspectedLimit":limit,"manifestPage":page,"nextManifestPage":if selected.is_none() && offset+limit<total {Some(page+1)}else{None},"remainingManifestCount":total.saturating_sub(offset+limit),"failures":failures,"source":source.source,"fetchedAt":source.fetched_at,"validatedAt":source.validated_at,"warning":if failures.is_empty(){Value::Null}else{json!({"kind":"PARTIAL","message":"Some manifests could not be inspected; existing data was retained."})}}),
        )
    }
    pub async fn refresh_repository_metadata(
        &self,
        op: &Operation,
        run: &str,
        ctx: &RequestContext,
    ) -> Result<Value, ApiError> {
        let names = if let Some(name) = op.repository.as_deref() {
            vec![normalize_repository(name)?]
        } else {
            self.http.db.gh_repository_names().await?
        };
        let mut results = Vec::new();
        let mut warning = Value::Null;
        for name in names {
            match self.get_repository_details(&name, op.force, ctx).await {
                Ok((repo, source)) => {
                    self.http.db.gh_event(run,None,"REPOSITORY_REFRESH",&json!({"repository":repo.full_name,"source":source.source,"fetchedAt":source.fetched_at,"validatedAt":source.validated_at,"archived":repo.archived})).await?;
                    results.push(json!({"repository":repo,"source":source.source,"fetchedAt":source.fetched_at,"validatedAt":source.validated_at}));
                    if let Some(error) = source.warning {
                        warning = json!(error);
                        break;
                    }
                }
                Err(error) => {
                    if error.kind == "NOT_FOUND" {
                        self.http.db.gh_unavailable(&name).await?;
                        self.http.db.gh_event(run,None,"REPOSITORY_UNAVAILABLE",&json!({"repository":name,"reason":"GitHub 404: deleted, private, renamed without redirect, or inaccessible"})).await?;
                    }
                    if error.kind == "CANCELLED" {
                        return Err(error);
                    }
                    warning = json!(error);
                    if ["RATE_LIMITED", "UNAUTHORIZED", "OFFLINE", "TIMEOUT"]
                        .contains(&error.kind.as_str())
                    {
                        break;
                    }
                }
            }
        }
        Ok(
            json!({"refreshedRepositories":results,"distinctRepositories":results.len(),"warning":warning}),
        )
    }
    pub(crate) async fn installation_tree(
        &self,
        repository: &str,
        commit: &str,
        root: &str,
        context: &RequestContext,
    ) -> Result<BTreeMap<String, (String, String)>, ApiError> {
        let (repo, outcome) = self
            .get_repository_details(repository, false, context)
            .await?;
        if let Some(error) = outcome.warning {
            return Err(error);
        }
        if repo.archived || repo.disabled {
            return Err(ApiError::new(
                "REVIEW_REQUIRED",
                "Archived/disabled sources require a new review.",
            ));
        }
        self.scoped_tree(&repo, commit, root, context).await
    }
    pub(crate) async fn installation_bytes(
        &self,
        repository: &str,
        path: &str,
        commit: &str,
        expected: &str,
        context: &RequestContext,
    ) -> Result<Vec<u8>, ApiError> {
        let repository = normalize_repository(repository)?;
        let path = safe_path(path)?;
        if commit.len() != 40
            || !commit.chars().all(|c| c.is_ascii_hexdigit())
            || expected.len() != 40
            || !expected.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(ApiError::new(
                "REVISION",
                "An immutable commit is required.",
            ));
        }
        let outcome = self
            .http
            .get(
                &endpoint(
                    &format!("/repos/{repository}/contents/{path}"),
                    &[("ref", commit)],
                ),
                365 * 86_400_000,
                false,
                context,
            )
            .await?;
        if let Some(error) = outcome.warning {
            return Err(error);
        }
        if outcome.body["type"] != "file"
            || outcome.body["path"] != path
            || outcome.body["sha"] != expected
        {
            return Err(ApiError::new(
                "UNSAFE_FILE",
                "Only ordinary pinned files may be downloaded.",
            ));
        }
        if outcome.body["size"]
            .as_u64()
            .is_none_or(|size| size > crate::installer::types::MAX_FILE_BYTES)
        {
            return Err(ApiError::new(
                "FILE_LIMIT",
                "The approved file exceeds the bounded package adapter limit.",
            ));
        }
        // Contents omits base64 for files over 1 MB. Fetch the already pinned
        // Git object, never an expiring download URL or a moving branch.
        let body = if outcome.body["encoding"] == "none" {
            let blob = self
                .http
                .get(
                    &format!("/repos/{repository}/git/blobs/{expected}"),
                    365 * 86_400_000,
                    false,
                    context,
                )
                .await?;
            if let Some(error) = blob.warning {
                return Err(error);
            }
            blob.body
        } else {
            outcome.body
        };
        if body["encoding"] != "base64" || body["sha"] != expected {
            return Err(ApiError::new(
                "INTEGRITY",
                "GitHub did not return the approved base64 blob.",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(
                body["content"]
                    .as_str()
                    .unwrap_or("")
                    .replace(['\n', '\r'], ""),
            )
            .map_err(|_| ApiError::new("DOWNLOAD", "Invalid content encoding."))?;
        if bytes.len() as u64 > crate::installer::types::MAX_FILE_BYTES
            || git_blob(&bytes) != expected
            || body["sha"] != expected
        {
            return Err(ApiError::new(
                "INTEGRITY",
                "Downloaded bytes do not match the approved blob or exceed the file limit.",
            ));
        }
        Ok(bytes)
    }
    pub async fn installation_plan(&self, id: &str) -> Result<Value, ApiError> {
        planner::plan(&self.http.db, id).await
    }
}
fn endpoint(path: &str, query: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("https://api.github.com").unwrap();
    url.set_path(path);
    url.query_pairs_mut().extend_pairs(query.iter().copied());
    format!(
        "{}{}",
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    )
}
pub(crate) fn git_blob(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", bytes.len()));
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}
