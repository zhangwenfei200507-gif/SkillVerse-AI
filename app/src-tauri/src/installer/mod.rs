mod config;
mod files;
mod planner;
mod probe;
mod sealed;
mod source;
mod store;
#[cfg(test)]
mod tests;
pub mod types;
use crate::{
    db::Database,
    github::{GitHubDiscoveryService, http::RequestContext},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use types::*;
pub struct Installer {
    db: Database,
    profile: Profile,
    provider: Arc<dyn source::PackageSource>,
    active: Mutex<Option<(String, RequestContext)>>,
    #[cfg(test)]
    fault: Mutex<Option<String>>,
}
struct ActiveLease<'a>(&'a Mutex<Option<(String, RequestContext)>>);
impl Drop for ActiveLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}
impl Installer {
    pub async fn new(
        db: Database,
        profile: Profile,
        github: Arc<GitHubDiscoveryService>,
    ) -> Result<Self> {
        Self::with_source(db, profile, Arc::new(source::GitHubSource(github))).await
    }
    async fn with_source(
        db: Database,
        profile: Profile,
        provider: Arc<dyn source::PackageSource>,
    ) -> Result<Self> {
        // Construction never creates or changes real Codex/project targets.
        if profile.isolated {
            for root in [
                &profile.user_home,
                &profile.codex_home,
                &profile.project_root,
            ] {
                files::guard(&profile, root)?;
                std::fs::create_dir_all(root).map_err(|_| {
                    fail(
                        "ISOLATION",
                        "The isolated environment could not be created.",
                    )
                })?;
            }
        }
        if let Ok(_lock) = files::exclusive_lock(&profile) {
            db.install_interrupt(&profile.id).await?;
        }
        Ok(Self {
            db,
            profile,
            provider,
            active: Mutex::new(None),
            #[cfg(test)]
            fault: Mutex::new(None),
        })
    }
    pub async fn state(&self) -> Result<Value> {
        let records = self.db.install_records(&self.profile.id).await?;
        let mut observed = Vec::new();
        for record in records {
            let mut value = json!(record);
            let mut state = record.state.clone();
            if state != "UNINSTALLED" {
                for owned in &record.targets {
                    match files::read(&self.profile, &owned.target)
                        .and_then(|tree| files::image(&tree))
                    {
                        Ok(actual) if actual == owned.image => {}
                        Ok(actual) if actual.kind == "absent" => state = "MISSING".into(),
                        _ => state = "LOCAL_MODIFIED".into(),
                    }
                }
                for owned in &record.patches {
                    let actual = files::read(&self.profile, &owned.target).and_then(|tree| {
                        config::entry(
                            &tree,
                            &owned.patch.format,
                            &owned.patch.section,
                            &owned.patch.key,
                        )
                    });
                    if !actual.is_ok_and(|entry| entry == owned.patch.value) {
                        state = "CONFIGURATION_CHANGED".into();
                    }
                }
            }
            value["observedState"] = json!(state);
            observed.push(value);
        }
        Ok(
            json!({"profile":self.profile,"records":observed,"history":self.db.install_history(&self.profile.id).await?,"runtimeExecutionSupported":false}),
        )
    }
    pub async fn scan(&self) -> Result<crate::codex_scanner::CodexScanReport> {
        self.db
            .require_identity_scan_ready()
            .await
            .map_err(Error::from)?;
        let _lock = files::exclusive_lock(&self.profile)?;
        let report = self.scan_report()?;
        self.db
            .persist_codex_scan(&report)
            .await
            .map_err(Error::from)?;
        Ok(report)
    }
    pub fn identity_context(&self) -> crate::capability_identity::ScanContext {
        crate::capability_identity::ScanContext::selected(
            self.profile.user_home.clone(),
            self.profile.codex_home.clone(),
            Some(self.profile.project_root.clone()),
        )
    }
    fn scan_report(&self) -> Result<crate::codex_scanner::CodexScanReport> {
        crate::capability_identity::scan(&self.identity_context())
            .map_err(|error| fail("IDENTITY_SCAN", &error))
    }
    pub async fn operation(&self, id: &str) -> Result<Value> {
        let op = self.db.install_operation(id).await?;
        if op.plan.profile.id != self.profile.id {
            return Err(fail("PROFILE", "Operation belongs to another environment."));
        }
        let progress:Option<String>=sqlx::query_scalar("SELECT details_json FROM installation_operation_events WHERE operation_id=? ORDER BY id DESC LIMIT 1").bind(id).fetch_optional(&self.db.pool).await.map_err(|_|Error::database())?;
        let mut value = json!(op);
        value["progress"] = progress
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        Ok(value)
    }
    pub async fn prepare(&self, request: Request) -> Result<Operation> {
        let context = RequestContext::new();
        let plan = planner::plan(
            &self.db,
            &self.profile,
            request,
            self.provider.as_ref(),
            &context,
        )
        .await?;
        self.db.install_plan(&plan).await
    }
    pub async fn cancel(&self, id: &str) -> Result<()> {
        {
            let active = self
                .active
                .lock()
                .map_err(|_| fail("BUSY", "Operation state is unavailable."))?;
            if let Some((current, context)) = active.as_ref() {
                if current == id {
                    context.cancel.cancel();
                    return Ok(());
                }
            }
        }
        let op = self.db.install_operation(id).await?;
        if op.plan.profile.id != self.profile.id {
            return Err(fail("PROFILE", "Operation belongs to another environment."));
        }
        if ["INTERRUPTED", "FAILED"].contains(&op.state.as_str())
            && op.recovery["writeIntent"].as_u64().is_some()
        {
            return Err(fail(
                "RECOVERY_REQUIRED",
                "Recover the interrupted changes before discarding this operation.",
            ));
        }
        if ["WAITING_FOR_APPROVAL", "INTERRUPTED", "FAILED"].contains(&op.state.as_str()) {
            self.db
                .install_event(id, "CANCELLED", &json!({"filesChanged":false}))
                .await?;
            self.db.install_unlock(id).await?;
        }
        Ok(())
    }
    fn check(context: &RequestContext) -> Result<()> {
        if context.cancel.is_cancelled() {
            Err(fail(
                "CANCELLED",
                "The operation was cancelled; owned changes will be restored.",
            ))
        } else {
            Ok(())
        }
    }
    fn trip(&self, _stage: &str) -> Result<()> {
        #[cfg(test)]
        {
            let mut fault = self.fault.lock().unwrap();
            if fault.as_deref() == Some(_stage)
                || fault.as_deref() == Some(&format!("crash-{_stage}"))
            {
                let crash = fault.as_ref().is_some_and(|v| v.starts_with("crash"));
                fault.take();
                return Err(fail(
                    if crash {
                        "INTERRUPTED"
                    } else {
                        "INJECTED_FAILURE"
                    },
                    "An isolated fault interrupted the transaction.",
                ));
            }
        }
        Ok(())
    }
    fn approve_plan(
        &self,
        op: &Operation,
        digest: &str,
        confirmed: bool,
        expiry: bool,
    ) -> Result<()> {
        if !confirmed
            || digest != op.digest
            || hash(
                serde_json::to_string(&op.plan)
                    .map_err(|_| fail("PLAN", "The saved plan is invalid."))?
                    .as_bytes(),
            ) != op.digest
        {
            return Err(fail(
                "APPROVAL",
                "Confirm the exact saved change preview before writing any files.",
            ));
        }
        if serde_json::to_value(&self.profile).ok() != serde_json::to_value(&op.plan.profile).ok() {
            return Err(fail(
                "PROFILE_CHANGED",
                "The target environment changed; generate a new preview.",
            ));
        }
        if !op.plan.can_execute || !op.plan.blockers.is_empty() {
            return Err(fail("BLOCKED", "Resolve all preview blockers first."));
        }
        if expiry && time() > op.plan.expires_at {
            return Err(fail(
                "STALE_PLAN",
                "This preview expired; review a new plan.",
            ));
        }
        Ok(())
    }
    pub async fn execute(&self, id: &str, digest: &str, confirmed: bool) -> Result<Operation> {
        let op = self.db.install_operation(id).await?;
        self.approve_plan(&op, digest, confirmed, true)?;
        if !["WAITING_FOR_APPROVAL", "INTERRUPTED", "FAILED"].contains(&op.state.as_str()) {
            return Err(fail(
                "STATE",
                "The operation is not awaiting approval or retry.",
            ));
        }
        if op.recovery["writeIntent"].as_u64().is_some() {
            return Err(fail(
                "RECOVERY_REQUIRED",
                "An interrupted write must be recovered before a new attempt.",
            ));
        }
        let _lock = files::exclusive_lock(&self.profile)?;
        let context = RequestContext::new();
        {
            let mut active = self
                .active
                .lock()
                .map_err(|_| fail("BUSY", "Operation state is unavailable."))?;
            if active.is_some() {
                return Err(fail("BUSY", "Another installation operation is running."));
            }
            *active = Some((id.into(), context.clone()));
        }
        let _lease = ActiveLease(&self.active);
        let result = async {
            self.db.install_reserve(&op).await?;
            self.run(&op, &context).await
        }
        .await;
        if let Err(error) = result {
            let _ = self.db.install_error(id, &error).await;
            if error.kind == "INTERRUPTED" {
                let _ = self
                    .db
                    .install_event(id, "INTERRUPTED", &json!({"recoveryRequired":true}))
                    .await;
            } else {
                let current = self.db.install_operation(id).await?;
                if current.recovery["writeIntent"].as_u64().is_some() {
                    if let Err(recovery) = self.restore(&current).await {
                        let _ = self.db.install_error(id, &recovery).await;
                        let _ = self
                            .db
                            .install_event(
                                id,
                                "INTERRUPTED",
                                &json!({"recoveryRequired":true,"reason":recovery.kind}),
                            )
                            .await;
                    } else {
                        self.db.install_unlock(id).await?;
                    }
                } else {
                    self.db
                        .install_event(
                            id,
                            if error.kind == "CANCELLED" {
                                "CANCELLED"
                            } else {
                                "FAILED"
                            },
                            &json!({"reason":error.kind,"retryAt":error.retry_at}),
                        )
                        .await?;
                    self.db.install_unlock(id).await?;
                }
            }
        }
        *self
            .active
            .lock()
            .map_err(|_| fail("BUSY", "Operation state is unavailable."))? = None;
        self.db.install_operation(id).await
    }
    async fn run(&self, op: &Operation, context: &RequestContext) -> Result<()> {
        let plan = &op.plan;
        if plan.action == "test-connection" {
            return self.probe(op, context).await;
        }
        let directory = self.profile.work_root.join(&op.id);
        files::guard(&self.profile, &directory)?;
        std::fs::create_dir_all(&directory)
            .map_err(|_| fail("STORAGE", "Operation storage could not be created."))?;
        Self::check(context)?;
        self.trip("disk-space")?;
        for c in &plan.changes {
            if files::image(&files::read(&self.profile, &c.target)?)? != c.before {
                return Err(fail(
                    "STALE_PLAN",
                    "Target files changed after preview. Generate a new plan.",
                ));
            }
        }
        let requires_source = plan.source.is_some()
            && ["install", "update", "configure"].contains(&plan.action.as_str());
        let mut package = None;
        if requires_source {
            let source = plan
                .source
                .as_ref()
                .ok_or_else(|| fail("SOURCE", "A reviewed source is required."))?;
            let (data, _) = source::reviewed(&self.db, &plan.entity_id).await?;
            if source::review_hash(&data) != source.review_hash {
                return Err(fail(
                    "REVIEW_CHANGED",
                    "Source review changed after preview. A new confirmation is required.",
                ));
            }
            self.db
                .install_event(
                    &op.id,
                    "DOWNLOADING",
                    &json!({"commit":source.commit,"files":source.files.len()}),
                )
                .await?;
            let mut tree = files::Tree {
                kind: "directory".into(),
                files: BTreeMap::new(),
            };
            let mut total = 0u64;
            for (index, file) in source.files.iter().enumerate() {
                if source.kind == "mcp" && file.path != source.manifest_path {
                    continue;
                }
                Self::check(context)?;
                let cached = directory.join("downloads").join(&file.relative);
                files::guard(&self.profile, &cached)?;
                let bytes = if cached.exists() {
                    let local = files::read(&self.profile, &cached)?;
                    STANDARD
                        .decode(local.files.get("").ok_or_else(|| {
                            fail("DOWNLOAD", "The download checkpoint is invalid.")
                        })?)
                        .map_err(|_| fail("DOWNLOAD", "Invalid checkpoint encoding."))?
                } else {
                    let bytes = self.provider.bytes(source, file, context).await?;
                    if crate::github::git_blob(&bytes) != file.blob
                        || bytes.len() as u64 > files::MAX_FILE
                    {
                        return Err(fail(
                            "INTEGRITY",
                            "The download differs from the approved blob.",
                        ));
                    }
                    files::durable(&cached, &bytes)?;
                    bytes
                };
                if crate::github::git_blob(&bytes) != file.blob {
                    return Err(fail(
                        "INTEGRITY",
                        "A staged download was altered. Nothing will be installed.",
                    ));
                }
                total += bytes.len() as u64;
                if total > files::MAX_TOTAL {
                    return Err(fail(
                        "LIMIT",
                        "The package exceeds its approved total byte limit.",
                    ));
                }
                if bytes.starts_with(b"MZ") || bytes.starts_with(b"\x7fELF") {
                    return Err(fail(
                        "EXECUTABLE",
                        "Executable binaries are not supported by this adapter.",
                    ));
                }
                let lower = file.relative.to_ascii_lowercase();
                let text_required = [
                    ".md", ".json", ".toml", ".py", ".js", ".mjs", ".ts", ".sh", ".ps1",
                ]
                .iter()
                .any(|ext| lower.ends_with(ext));
                if let Ok(raw) = std::str::from_utf8(&bytes) {
                    let findings = crate::github::analysis::risks(raw, &[file.path.clone()]);
                    if !findings.is_empty() {
                        let (mut assessment, _) =
                            source::reviewed(&self.db, &plan.entity_id).await?;
                        assessment["reviewState"] = json!("NEEDS_REVIEW");
                        assessment["risks"] = json!(findings);
                        sqlx::query("UPDATE github_capability_intelligence SET payload_json=?,checked_at=? WHERE entity_id=?").bind(assessment.to_string()).bind(time()).bind(&plan.entity_id).execute(&self.db.pool).await.map_err(|_|Error::database())?;
                        return Err(fail(
                            "REVIEW_REQUIRED",
                            "Downloaded package content introduced a static security finding. Review the source and generate a new preview.",
                        ));
                    }
                    if file.path.ends_with("SKILL.md") {
                        crate::github::analysis::skill_metadata(raw)?;
                    }
                } else if text_required {
                    return Err(fail(
                        "CONTENT",
                        "A declared text/script file is not valid UTF-8.",
                    ));
                }
                tree.files
                    .insert(file.relative.clone(), STANDARD.encode(bytes));
                self.db.install_event(&op.id,"DOWNLOADING",&json!({"completedFiles":index+1,"totalFiles":source.files.len(),"bytes":total,"path":file.relative})).await?;
            }
            self.db
                .install_event(
                    &op.id,
                    "STAGING",
                    &json!({"completeFiles":tree.files.len()}),
                )
                .await?;
            Self::check(context)?;
            self.db
                .install_event(&op.id, "VALIDATING", &json!({"immutableSource":true}))
                .await?;
            if source.kind != "mcp" {
                let inventory = source
                    .files
                    .iter()
                    .map(|file| (file.path.clone(), (file.blob.clone(), file.mode.clone())))
                    .collect();
                if crate::github::analysis::package_fingerprint(&source.package_root, &inventory)
                    .as_deref()
                    != Some(&source.fingerprint)
                {
                    return Err(fail("INTEGRITY", "The source inventory changed."));
                }
            }
            let manifest = source
                .files
                .iter()
                .find(|f| f.path == source.manifest_path)
                .ok_or_else(|| fail("MANIFEST", "Missing manifest."))?;
            let raw =
                String::from_utf8(
                    STANDARD
                        .decode(tree.files.get(&manifest.relative).ok_or_else(|| {
                            fail("MANIFEST", "The downloaded manifest is absent.")
                        })?)
                        .map_err(|_| fail("MANIFEST", "Manifest encoding is invalid."))?,
                )
                .map_err(|_| fail("MANIFEST", "Manifest must be UTF-8."))?;
            if source.kind == "skill" {
                if crate::github::analysis::skill_metadata(&raw)?.0 != source.name {
                    return Err(fail(
                        "MANIFEST",
                        "The Skill name differs from the approved manifest.",
                    ));
                }
            } else if source.kind == "plugin" {
                let manifest: Value = serde_json::from_str(&raw)
                    .map_err(|_| fail("MANIFEST", "Plugin manifest JSON is invalid."))?;
                if manifest["name"] != source.name
                    || manifest.get("hooks").is_some()
                    || manifest["extensions"]["com.openai"].get("hooks").is_some()
                {
                    return Err(fail(
                        "HOOK_REVIEW",
                        "The plugin identifier or lifecycle declaration needs a new review.",
                    ));
                }
            }
            package = Some(tree);
        }
        let mut after_trees = Vec::new();
        for c in &plan.changes {
            Self::check(context)?;
            let current = files::read(&self.profile, &c.target)?;
            if files::image(&current)? != c.before {
                return Err(fail(
                    "STALE_PLAN",
                    "Targets changed during download; a new preview is required.",
                ));
            }
            let after = if let Some(patch) = c.patch.as_ref() {
                config::apply(&current, patch)?
            } else if let Some(snapshot) = c.restore_from.as_ref() {
                let path = self.profile.work_root.join(files::relative(snapshot)?);
                files::guard(&self.profile, &path)?;
                sealed::load(&path)?
            } else if c.role == "remove-owned" {
                files::missing()
            } else {
                package
                    .clone()
                    .ok_or_else(|| fail("STAGING", "A complete package is unavailable."))?
            };
            after_trees.push(after);
        }
        self.db
            .install_event(
                &op.id,
                "BACKING_UP",
                &json!({"targets":plan.changes.len(),"encryption":"Windows DPAPI current user"}),
            )
            .await?;
        for (index, c) in plan.changes.iter().enumerate() {
            Self::check(context)?;
            let before = files::read(&self.profile, &c.target)?;
            if files::image(&before)? != c.before {
                return Err(fail("STALE_PLAN", "Target changed before backup."));
            }
            for (label, tree) in [("before", &before), ("after", &after_trees[index])] {
                let path = directory.join(format!("{label}-{index}.sealed"));
                if path.exists() {
                    if files::image(&sealed::load(&path)?)? != files::image(tree)? {
                        return Err(fail(
                            "BACKUP",
                            "A prior recovery artifact does not match this operation.",
                        ));
                    }
                } else {
                    sealed::save(&path, tree)?;
                }
            }
        }
        let database_backup = directory.join("database.sealed");
        if !database_backup.exists() {
            let bytes =
                serde_json::to_vec(&self.db.install_database_snapshot(&plan.entity_id).await?)
                    .map_err(|_| {
                        fail("BACKUP", "Database metadata snapshot could not be encoded.")
                    })?;
            sealed::save(&database_backup, &files::file_tree(&bytes))?;
        }
        let _ = sealed::load(&database_backup)?;
        let after_images = after_trees
            .iter()
            .map(files::image)
            .collect::<Result<Vec<_>>>()?;
        let mut recovery = json!({"backupVerified":true,"after":after_images,"completedSteps":0});
        self.db.install_recovery(&op.id, &recovery).await?;
        self.trip("after-backup")?;
        self.db
            .install_event(&op.id, "INSTALLING", &json!({"backupVerified":true}))
            .await?;
        for (index, c) in plan.changes.iter().enumerate() {
            Self::check(context)?;
            if files::image(&files::read(&self.profile, &c.target)?)? != c.before {
                return Err(fail("LOCAL_MODIFIED", "A target changed before its write."));
            }
            files::space(&c.target, files::MAX_TOTAL * 2)?;
            if c.staging_path.exists() || c.holding_path.exists() {
                return Err(fail(
                    "RECOVERY_REQUIRED",
                    "A same-volume recovery path unexpectedly exists.",
                ));
            }
            if let Some(parent) = c.holding_path.parent() {
                files::guard(&self.profile, parent)?;
                std::fs::create_dir_all(parent).map_err(|_| {
                    fail(
                        "PERMISSION",
                        "The same-volume transaction directory is unavailable.",
                    )
                })?;
            }
            files::write_new(&self.profile, &c.staging_path, &after_trees[index])?;
            recovery["writeIntent"] = json!(index);
            self.db.install_recovery(&op.id, &recovery).await?;
            if c.before.kind != "absent" {
                files::rename(&self.profile, &c.target, &c.holding_path)?;
            }
            self.trip(&format!("crash-old-{index}"))?;
            if after_trees[index].kind != "absent" {
                if let Some(parent) = c.target.parent() {
                    files::guard(&self.profile, parent)?;
                    std::fs::create_dir_all(parent).map_err(|_| {
                        fail("PERMISSION", "The installation parent is unavailable.")
                    })?;
                }
                files::rename(&self.profile, &c.staging_path, &c.target)?;
            }
            recovery["completedSteps"] = json!(index + 1);
            self.db.install_recovery(&op.id, &recovery).await?;
            self.trip(&format!("after-write-{index}"))?;
            #[cfg(debug_assertions)]
            if self.profile.isolated
                && std::env::var("SKILLVERSE_ISOLATED_PAUSE_AFTER_STEP")
                    .ok()
                    .as_deref()
                    == Some(&index.to_string())
            {
                tokio::time::sleep(std::time::Duration::from_secs(45)).await;
            }
            if files::image(&files::read(&self.profile, &c.target)?)? != after_images[index] {
                return Err(fail(
                    "VERIFY",
                    "An installed target failed byte verification.",
                ));
            }
        }
        Self::check(context)?;
        self.db
            .install_event(
                &op.id,
                "VERIFYING",
                &json!({"filesMatchApprovedSource":true}),
            )
            .await?;
        let report = self.scan_report()?;
        let record = self.make_record(op, &after_images, &report).await?;
        recovery["scanAfterAt"] = json!(report.scanned_at_unix_ms);
        self.db.install_recovery(&op.id, &recovery).await?;
        self.trip("before-db")?;
        self.commit(op, record.as_ref(), &report).await?;
        self.trip("after-db")?;
        #[cfg(test)]
        {
            let corrupt = self.fault.lock().unwrap().as_deref() == Some("corrupt-after-db");
            if corrupt {
                self.fault.lock().unwrap().take();
                if let Some(source) = plan.source.as_ref() {
                    std::fs::write(
                        plan.changes[0].target.join(&source.files[0].relative),
                        b"concurrent external file edit",
                    )
                    .unwrap();
                }
            }
        }
        for (index, c) in plan.changes.iter().enumerate() {
            if files::image(&files::read(&self.profile, &c.target)?)? != after_images[index] {
                return Err(fail(
                    "VERIFY",
                    "Files changed during the final database commit; recovery is required.",
                ));
            }
        }
        self.db.install_event(&op.id,"COMPLETED",&json!({"filesVerified":true,"scannerObserved":record.as_ref().is_some_and(|r|r.verification.iter().any(|v|v=="DISCOVERED_BY_SCANNER")),"runtimeVerified":false})).await?;
        self.db.install_unlock(&op.id).await?;
        self.cleanup_holding(op);
        Ok(())
    }
    async fn make_record(
        &self,
        op: &Operation,
        after: &[Image],
        report: &crate::codex_scanner::CodexScanReport,
    ) -> Result<Option<Record>> {
        let p = &op.plan;
        if p.action == "rollback" {
            let previous = self
                .db
                .install_operation(
                    p.rollback_operation
                        .as_deref()
                        .ok_or_else(|| fail("ROLLBACK", "Missing rollback parent."))?,
                )
                .await?;
            return Ok(previous.plan.prior_record.map(|mut r| {
                r.current_operation_id = op.id.clone();
                r
            }));
        }
        let primary = p
            .changes
            .iter()
            .find(|c| c.role == "package")
            .map(|c| c.target.clone())
            .or_else(|| p.prior_record.as_ref().map(|r| r.target.clone()))
            .unwrap_or_else(|| p.changes[0].target.clone());
        let mut r = p.prior_record.clone().unwrap_or(Record {
            id: id(),
            entity_id: p.entity_id.clone(),
            profile_id: self.profile.id.clone(),
            kind: p.kind.clone(),
            scope: p.scope.clone(),
            target: primary,
            source: p.source.clone(),
            state: String::new(),
            verification: vec![],
            local_entity_id: None,
            current_operation_id: op.id.clone(),
            targets: vec![],
            patches: vec![],
        });
        r.current_operation_id = op.id.clone();
        r.source = p.source.clone();
        if p.action == "uninstall" {
            r.state = "UNINSTALLED".into();
            r.verification = vec!["OWNED_FILES_REMOVED".into()];
            return Ok(Some(r));
        }
        if !["enable", "disable"].contains(&p.action.as_str()) {
            r.targets = p
                .changes
                .iter()
                .zip(after)
                .filter(|(c, _)| c.role == "package" || c.role == "package-cache")
                .map(|(c, image)| OwnedTarget {
                    target: c.target.clone(),
                    image: image.clone(),
                })
                .collect();
            r.patches = p
                .changes
                .iter()
                .filter_map(|c| {
                    c.patch.as_ref().map(|patch| OwnedPatch {
                        target: c.target.clone(),
                        patch: patch.clone(),
                    })
                })
                .collect();
        } else {
            for c in &p.changes {
                if let Some(patch) = c.patch.as_ref() {
                    if let Some(owned) = r
                        .patches
                        .iter_mut()
                        .find(|owned| owned.target == c.target && owned.patch.key == patch.key)
                    {
                        owned.patch = patch.clone();
                    }
                }
            }
        }
        r.verification = vec![];
        if p.kind == "mcp" {
            r.state = "CONFIGURED".into();
            r.verification.push("CONFIGURED".into());
            r.verification.push("CONNECTION_NOT_TESTED".into());
            if p.requirements
                .iter()
                .any(|value| value.starts_with("AUTH_REQUIRED"))
            {
                r.verification.push("AUTH_REQUIRED".into());
            }
            for owned in &r.patches {
                if let Some(value) = owned.patch.value.as_ref() {
                    if value.get("bearer_token_env_var").is_some() {
                        r.verification.push("AUTH_REQUIRED".into());
                    }
                    if let Some(command) = value["command"].as_str() {
                        if !runtime_exists(command) {
                            r.verification.push("DEPENDENCIES_MISSING".into());
                        }
                    }
                    if value["env_vars"].as_array().is_some_and(|names| {
                        names
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|name| std::env::var_os(name).is_none_or(|value| value.is_empty()))
                    }) {
                        r.verification.push("DEPENDENCIES_MISSING".into());
                        r.verification.push("CONFIGURATION_REQUIRED".into());
                    }
                }
            }
        } else {
            r.state = "FILES_INSTALLED".into();
            r.verification.push("FILES_INSTALLED".into());
            let local = if p.kind == "skill" {
                r.target
                    .join("SKILL.md")
                    .canonicalize()
                    .ok()
                    .map(|path| crate::codex_scanner::stable_id("skill", path))
            } else {
                r.targets
                    .iter()
                    .find(|t| {
                        t.target
                            .starts_with(self.profile.codex_home.join("plugins/cache"))
                    })
                    .map(|t| {
                        let mut actual = self.profile.codex_home.clone();
                        if let Ok(relative) = t.target.strip_prefix(&self.profile.codex_home) {
                            for component in relative.components() {
                                actual.push(component);
                            }
                        }
                        crate::codex_scanner::stable_id("plugin", actual)
                    })
            };
            if let Some(local) = local {
                if report.entities.iter().any(|entity| entity.id == local) {
                    r.local_entity_id = Some(local);
                    r.verification.push("DISCOVERED_BY_SCANNER".into());
                } else {
                    return Err(fail(
                        "SCANNER_VERIFY",
                        "The read-only scanner did not find the installed package.",
                    ));
                }
            }
            if p.kind == "plugin" {
                r.verification
                    .extend(["DOWNLOADED".into(), "REGISTERED".into()]);
            }
        }
        if r.patches.iter().any(|owned| {
            owned
                .patch
                .value
                .as_ref()
                .is_some_and(|v| v["enabled"] == true)
        }) {
            r.verification.push("ENABLED_POLICY".into());
        } else if p.kind != "skill" {
            r.verification.push("DISABLED_POLICY".into());
        }
        if p.kind == "mcp"
            && !r
                .verification
                .iter()
                .any(|v| v == "AUTH_REQUIRED" || v == "DEPENDENCIES_MISSING")
        {
            r.verification.push("READY_TO_TEST".into());
        }
        if !p.requirements.is_empty() {
            r.verification.push("CONFIGURATION_REQUIRED".into());
        }
        Ok(Some(r))
    }
    async fn commit(
        &self,
        op: &Operation,
        record: Option<&Record>,
        report: &crate::codex_scanner::CodexScanReport,
    ) -> Result<()> {
        let mut tx = self.db.pool.begin().await.map_err(|_| Error::database())?;
        let mut marked = report.clone();
        if let Some(identity) = marked.identity.as_mut() {
            identity.operation_id = Some(op.id.clone());
        }
        for entity in &mut marked.entities {
            entity.evidence["installationOperationId"] = json!(op.id);
        }
        self.db
            .persist_codex_scan_in(&mut tx, &marked)
            .await
            .map_err(Error::from)?;
        if let Some(record) = record {
            let key = hash(
                format!(
                    "{}|{}",
                    record.target.to_string_lossy().to_lowercase(),
                    if record.kind == "mcp" {
                        record
                            .patches
                            .first()
                            .map(|p| p.patch.key.as_str())
                            .unwrap_or("")
                    } else {
                        ""
                    }
                )
                .as_bytes(),
            );
            sqlx::query("INSERT INTO managed_installations(id,entity_id,profile_id,kind,scope,target_key,record_json,state,current_operation_id,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET target_key=excluded.target_key,record_json=excluded.record_json,state=excluded.state,current_operation_id=excluded.current_operation_id,updated_at=excluded.updated_at").bind(&record.id).bind(&record.entity_id).bind(&record.profile_id).bind(&record.kind).bind(&record.scope).bind(key).bind(json!(record).to_string()).bind(&record.state).bind(&record.current_operation_id).bind(time()).execute(&mut *tx).await.map_err(|_|Error::database())?;
        } else if let Some(prior) = op.plan.prior_record.as_ref() {
            sqlx::query(
                "DELETE FROM managed_installations WHERE id=? AND current_operation_id IN (?,?)",
            )
            .bind(&prior.id)
            .bind(&op.id)
            .bind(&prior.current_operation_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::database())?;
        }
        if record.is_none() && op.plan.prior_record.is_none() {
            sqlx::query("DELETE FROM managed_installations WHERE current_operation_id=?")
                .bind(&op.id)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::database())?;
        }
        let any_active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM managed_installations WHERE entity_id=? AND state!='UNINSTALLED'",
        )
        .bind(&op.plan.entity_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Error::database())?;
        let status = if any_active == 0 {
            "NOT_INSTALLED"
        } else if op.plan.kind == "mcp" {
            "UNKNOWN"
        } else {
            "INSTALLED"
        };
        sqlx::query("INSERT INTO installations(id,entity_id,status,source,version,observed_at,evidence_json) VALUES(?,?,?,'app',?,?,?) ON CONFLICT(entity_id) DO UPDATE SET status=excluded.status,version=excluded.version,observed_at=excluded.observed_at,evidence_json=excluded.evidence_json,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')").bind(format!("managed:{}",op.plan.entity_id)).bind(&op.plan.entity_id).bind(status).bind(record.and_then(|r|r.source.as_ref()).map(|s|s.commit.as_str())).bind(time().to_string()).bind(json!({"operationId":op.id,"profileId":self.profile.id,"filesVerified":any_active>0,"runtimeVerified":false}).to_string()).execute(&mut *tx).await.map_err(|_|Error::database())?;
        tx.commit().await.map_err(|_| Error::database())?;
        Ok(())
    }
    fn cleanup_holding(&self, op: &Operation) {
        for c in &op.plan.changes {
            if c.holding_path.exists() {
                let _ = files::remove_owned(&self.profile, &c.holding_path, &c.before);
            }
        }
    }
    pub async fn recover(&self, id: &str, digest: &str, confirmed: bool) -> Result<Operation> {
        let op = self.db.install_operation(id).await?;
        self.approve_plan(&op, digest, confirmed, false)?;
        if !["INTERRUPTED", "FAILED"].contains(&op.state.as_str()) {
            return Err(fail(
                "RECOVERY",
                "Only interrupted/failed operations can use recovery.",
            ));
        }
        let _lock = files::exclusive_lock(&self.profile)?;
        if let Err(error) = self.restore(&op).await {
            let _ = self.db.install_error(id, &error).await;
            let _ = self
                .db
                .install_event(id, "INTERRUPTED", &json!({"recoveryRequired":true}))
                .await;
            return Err(error);
        }
        self.db.install_unlock(id).await?;
        self.db.install_operation(id).await
    }
    async fn restore(&self, op: &Operation) -> Result<()> {
        if op.recovery["backupVerified"] != true {
            return Err(fail(
                "BACKUP",
                "No verified before-write snapshots exist; no recovery files were applied.",
            ));
        }
        self.db
            .install_event(&op.id, "ROLLING_BACK", &json!({"ownedChangesOnly":true}))
            .await?;
        for (index, c) in op.plan.changes.iter().enumerate().rev() {
            let before = sealed::load(
                &self
                    .profile
                    .work_root
                    .join(&op.id)
                    .join(format!("before-{index}.sealed")),
            )?;
            let after = sealed::load(
                &self
                    .profile
                    .work_root
                    .join(&op.id)
                    .join(format!("after-{index}.sealed")),
            )?;
            let current = files::read(&self.profile, &c.target)?;
            let current_image = files::image(&current)?;
            if current_image == c.before {
                continue;
            }
            let desired = if current_image == files::image(&after)? || current.kind == "absent" {
                before
            } else if let Some(patch) = c.patch.as_ref() {
                let written = config::entry(&after, &patch.format, &patch.section, &patch.key)?;
                if config::entry(&current, &patch.format, &patch.section, &patch.key)? != written {
                    return Err(fail(
                        "ROLLBACK_CONFLICT",
                        "An affected entry changed; recovery will not overwrite it.",
                    ));
                }
                let old_value = config::entry(&before, &patch.format, &patch.section, &patch.key)?;
                config::apply(
                    &current,
                    &config::new_patch(
                        &current,
                        &patch.format,
                        &patch.section,
                        &patch.key,
                        old_value,
                    )?,
                )?
            } else {
                return Err(fail(
                    "LOCAL_MODIFIED",
                    "Recovery found user modifications and stopped.",
                ));
            };
            // Drop only a byte-verified current image, then restore from verified snapshot.
            // Recovery intent remains durable if either transition is interrupted.
            if current.kind != "absent" {
                files::remove_owned(&self.profile, &c.target, &current_image)?;
            }
            let scratch = files::scratch(&self.profile, &c.target, &op.id, index, "recovery")?;
            if scratch.exists() {
                let actual = files::image(&files::read(&self.profile, &scratch)?)?;
                if actual != files::image(&desired)? {
                    return Err(fail(
                        "ROLLBACK_CONFLICT",
                        "A recovery staging path differs from its snapshot.",
                    ));
                }
            } else {
                files::write_new(&self.profile, &scratch, &desired)?;
            }
            if desired.kind != "absent" {
                if let Some(parent) = c.target.parent() {
                    std::fs::create_dir_all(parent).map_err(|_| {
                        fail("PERMISSION", "The recovery parent cannot be created.")
                    })?;
                }
                files::rename(&self.profile, &scratch, &c.target)?;
            }
            self.trip(&format!("crash-rollback-{index}"))?;
        }
        let backup = sealed::load(&self.profile.work_root.join(&op.id).join("database.sealed"))?;
        let before: Value = serde_json::from_str(&files::text(&backup)?)
            .map_err(|_| fail("BACKUP", "Database recovery snapshot is invalid."))?;
        self.db.install_restore_database(op, &before).await?;
        self.db
            .install_event(
                &op.id,
                "ROLLED_BACK",
                &json!({"originalBytesRestored":true,"runtimeVerified":false}),
            )
            .await?;
        self.cleanup_holding(op);
        Ok(())
    }
    pub async fn verify(&self, entity: &str) -> Result<Value> {
        let record = self
            .db
            .install_records(&self.profile.id)
            .await?
            .into_iter()
            .find(|r| r.entity_id == entity && r.state != "UNINSTALLED")
            .ok_or_else(|| fail("NOT_FOUND", "No active managed installation exists."))?;
        let mut findings = Vec::new();
        for owned in &record.targets {
            findings.push(json!({"target":owned.target,"matchesOwnedBytes":files::image(&files::read(&self.profile,&owned.target)?)?==owned.image}));
        }
        for owned in &record.patches {
            findings.push(json!({"target":owned.target,"entry":owned.patch.key,"matchesOwnedEntry":config::entry(&files::read(&self.profile,&owned.target)?,&owned.patch.format,&owned.patch.section,&owned.patch.key)?==owned.patch.value}));
        }
        Ok(json!({"record":record,"findings":findings,"runtimeVerified":false}))
    }
}
fn runtime_exists(command: &str) -> bool {
    if command.contains(['\\', '/']) {
        return !command.starts_with("\\\\")
            && !command.starts_with("//")
            && std::path::Path::new(command).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .filter(|path| !path.to_string_lossy().starts_with("\\\\"))
            .any(|path| {
                [".exe", ".cmd", ""]
                    .iter()
                    .any(|ext| path.join(format!("{command}{ext}")).is_file())
            })
    })
}
