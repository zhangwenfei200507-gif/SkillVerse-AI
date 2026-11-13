use super::{config, files, source, types::*};
use crate::{db::Database, github::http::RequestContext};
use serde_json::{Value, json};
use std::path::PathBuf;
fn change(profile: &Profile, target: PathBuf, role: &str, patch: Option<Patch>) -> Result<Change> {
    files::guard(profile, &target)?;
    let tree = files::read(profile, &target)?;
    if std::fs::metadata(&target).is_ok_and(|meta| meta.permissions().readonly()) {
        return Err(fail(
            "PERMISSION",
            "An affected target is read-only; it was not modified.",
        ));
    }
    Ok(Change {
        target,
        staging_path: PathBuf::new(),
        holding_path: PathBuf::new(),
        role: role.into(),
        before: files::image(&tree)?,
        patch,
        restore_from: None,
    })
}
pub async fn plan(
    db: &Database,
    profile: &Profile,
    request: Request,
    provider: &dyn source::PackageSource,
    context: &RequestContext,
) -> Result<Plan> {
    if !["user", "project"].contains(&request.scope.as_str()) {
        return Err(fail(
            "SCOPE",
            "Choose user or project scope. Bundled Skills are installed through their Plugin.",
        ));
    }
    let id = id();
    let now = time();
    let records = db.install_records(&profile.id).await?;
    let existing = records
        .into_iter()
        .find(|r| r.entity_id == request.entity_id && r.scope == request.scope);
    let active = existing.as_ref().filter(|r| r.state != "UNINSTALLED");
    let mut p = Plan {
        operation_id: id,
        profile: profile.clone(),
        entity_id: request.entity_id.clone(),
        action: request.action.clone(),
        scope: request.scope.clone(),
        kind: String::new(),
        source: None,
        source_changes: vec![],
        managed_id: existing.as_ref().map(|r| r.id.clone()),
        changes: vec![],
        blockers: vec![],
        warnings: vec![],
        requirements: vec![],
        prior_record: existing.clone(),
        created_at: now,
        expires_at: now + 30 * 60_000,
        rollback_operation: request.rollback_operation.clone(),
        dry_run: false,
        can_execute: false,
    };
    if request.action == "rollback" {
        return finalize(rollback_plan(db, p).await?);
    }
    if request.action == "test-connection" {
        let record = active.ok_or_else(|| {
            fail(
                "OWNERSHIP",
                "Configure an owned MCP before requesting a connection test.",
            )
        })?;
        if record.kind != "mcp" {
            return Err(fail(
                "UNSUPPORTED",
                "Connection tests apply only to managed MCP declarations.",
            ));
        }
        let owned = record
            .patches
            .iter()
            .find(|patch| patch.patch.section == "mcp_servers")
            .ok_or_else(|| fail("CONFIG", "The managed MCP declaration is missing."))?;
        let tree = files::read(profile, &owned.target)?;
        if config::entry(&tree, "toml", "mcp_servers", &owned.patch.key)? != owned.patch.value {
            return Err(fail(
                "CONFIG_CONFLICT",
                "The managed MCP declaration changed. Review it before testing.",
            ));
        }
        let value = owned
            .patch
            .value
            .as_ref()
            .ok_or_else(|| fail("CONFIG", "The MCP entry is missing."))?;
        let url = value["url"].as_str().ok_or_else(|| fail("RUNTIME_NOT_SUPPORTED", "SkillVerse does not launch stdio executables. Test that service explicitly in Codex."))?;
        p.kind = "mcp".into();
        p.changes.push(change(
            profile,
            owned.target.clone(),
            "observe-only",
            Some(owned.patch.clone()),
        )?);
        p.warnings.push(format!("Separately authorized network test: POST initialize, notifications/initialized and tools/list to {url}; optionally DELETE the negotiated session. MCP 2025-11-25 compatibility mode. No tool calls, subprocesses or configuration writes. Redirects are refused; each response is bounded to 256 KiB and 8 seconds."));
        if let Some(name) = value["bearer_token_env_var"].as_str() {
            p.requirements.push(format!("Read environment variable {name} only for the approved HTTPS endpoint. No credential is logged or saved."));
        }
        p.can_execute = true;
        return finalize(p);
    }
    if ["uninstall", "disable", "enable"].contains(&request.action.as_str()) {
        let record = active.ok_or_else(|| {
            fail(
                "OWNERSHIP",
                "No active SkillVerse-owned installation exists in this scope.",
            )
        })?;
        p.kind = record.kind.clone();
        p.source = record.source.clone();
        if request.action == "uninstall" {
            for owned in &record.targets {
                let c = change(profile, owned.target.clone(), "remove-owned", None)?;
                if c.before != owned.image {
                    return Err(fail(
                        "LOCAL_MODIFIED",
                        "Owned files were modified or extra files were added. Uninstall was stopped.",
                    ));
                }
                p.changes.push(c);
            }
            for owned in &record.patches {
                let tree = files::read(profile, &owned.target)?;
                let current = config::entry(
                    &tree,
                    &owned.patch.format,
                    &owned.patch.section,
                    &owned.patch.key,
                )?;
                if current != owned.patch.value {
                    return Err(fail(
                        "CONFIG_CONFLICT",
                        "A managed registration/configuration entry was edited; it will not be removed.",
                    ));
                }
                let patch = config::new_patch(
                    &tree,
                    &owned.patch.format,
                    &owned.patch.section,
                    &owned.patch.key,
                    None,
                )?;
                p.changes.push(change(
                    profile,
                    owned.target.clone(),
                    "configuration",
                    Some(patch),
                )?);
            }
        } else {
            if record.kind == "skill" {
                return Err(fail(
                    "UNSUPPORTED",
                    "Skill activation is controlled by Codex, not inferred from installed files.",
                ));
            }
            let owned = record
                .patches
                .iter()
                .find(|v| v.patch.format == "toml")
                .ok_or_else(|| fail("CONFIG", "No managed enable/disable policy exists."))?;
            let tree = files::read(profile, &owned.target)?;
            let mut value =
                config::entry(&tree, "toml", &owned.patch.section, &owned.patch.key)?
                    .ok_or_else(|| fail("CONFIG", "The managed policy entry is missing."))?;
            if Some(&value) != owned.patch.value.as_ref() {
                return Err(fail(
                    "CONFIG_CONFLICT",
                    "The managed policy changed outside SkillVerse.",
                ));
            }
            value["enabled"] = json!(request.action == "enable");
            let patch = config::new_patch(
                &tree,
                "toml",
                &owned.patch.section,
                &owned.patch.key,
                Some(value),
            )?;
            p.changes.push(change(
                profile,
                owned.target.clone(),
                "configuration",
                Some(patch),
            )?);
            p.warnings.push(if request.action=="enable" {"Enabling policy can let Codex start bundled services/hooks when it next loads the capability. This is a separate approval; SkillVerse itself will not launch them."} else {"Disables the managed load policy. This does not terminate a service already running in another Codex session."}.into());
            if record.kind == "plugin"
                && record.source.as_ref().is_some_and(|s| {
                    s.files
                        .iter()
                        .any(|f| f.relative.to_lowercase().contains("hooks"))
                })
            {
                return Err(fail(
                    "HOOK_REVIEW",
                    "Plugin hooks remain blocked pending a dedicated reviewed execution flow.",
                ));
            }
        }
    } else if ["install", "update", "configure"].contains(&request.action.as_str()) {
        if request.action == "install" && active.is_some() {
            p.blockers.push(
                "Already managed in this scope. Verify the existing installation or choose Update."
                    .into(),
            );
        }
        if request.action == "update" && active.is_none() {
            return Err(fail(
                "OWNERSHIP",
                "Only an existing managed installation can be updated.",
            ));
        }
        let kind = db
            .get_capability(&request.entity_id)
            .await
            .map_err(Error::from)?
            .ok_or_else(|| fail("NOT_FOUND", "Capability not found."))?
            .kind;
        if kind == "mcp" {
            let draft = request.mcp.as_ref().ok_or_else(|| {
                fail(
                    "CONFIG_REQUIRED",
                    "Provide a transport declaration to review before configuration.",
                )
            })?;
            p.kind = "mcp".into();
            p.source = None;
            let target = if request.scope == "project" {
                profile.project_root.join(".codex/config.toml")
            } else {
                profile.codex_home.join("config.toml")
            };
            let tree = files::read(profile, &target)?;
            let value = config::mcp_value(draft)?;
            let previous = config::entry(&tree, "toml", "mcp_servers", &draft.name)?;
            if previous.is_some() && active.is_none() {
                return Err(fail(
                    "NAME_CONFLICT",
                    "An unmanaged MCP with this name already exists. It will not be overwritten.",
                ));
            }
            if let Some(record) = active {
                if !record.patches.iter().any(|owned| {
                    owned.target == target
                        && owned.patch.key == draft.name
                        && owned.patch.value == previous
                }) {
                    return Err(fail(
                        "CONFIG_CONFLICT",
                        "The owned MCP declaration changed or the name differs.",
                    ));
                }
            }
            let patch = config::new_patch(&tree, "toml", "mcp_servers", &draft.name, Some(value))?;
            p.changes
                .push(change(profile, target, "configuration", Some(patch))?);
            if draft.auth_required {
                p.requirements.push(
                    "AUTH_REQUIRED: an external account or credentials require separate setup."
                        .into(),
                );
            }
            p.warnings.push("This approves a structured MCP declaration only. No repository package is downloaded, no command is started and no connection is inferred. The declaration digest fixes the exact configuration.".into());
            p.can_execute = p.blockers.is_empty();
            return finalize(p);
        }
        let source = source::snapshot(db, &request.entity_id, provider, context).await?;
        p.kind = source.kind.clone();
        p.requirements = source.requirements.clone();
        let old_files: std::collections::BTreeMap<_, _> = active
            .and_then(|record| record.source.as_ref())
            .map(|previous| {
                previous
                    .files
                    .iter()
                    .map(|file| (file.relative.clone(), file.blob.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let new_files: std::collections::BTreeMap<_, _> = source
            .files
            .iter()
            .map(|file| (file.relative.clone(), file.blob.clone()))
            .collect();
        for path in old_files
            .keys()
            .chain(new_files.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let state = match (old_files.get(path), new_files.get(path)) {
                (None, Some(_)) => "added",
                (Some(_), None) => "removed",
                (Some(before), Some(after)) if before != after => "changed",
                _ => continue,
            };
            let script = [".py", ".js", ".mjs", ".ts", ".sh", ".ps1"]
                .iter()
                .any(|ext| path.to_lowercase().ends_with(ext));
            p.source_changes.push(json!({"path":path,"state":state,"script":script,"beforeBlob":old_files.get(path),"afterBlob":new_files.get(path)}));
        }
        for requirement in &source.requirements {
            if active
                .and_then(|r| r.source.as_ref())
                .is_none_or(|previous| !previous.requirements.contains(requirement))
            {
                p.warnings
                    .push(format!("New source requirement to review: {requirement}"));
            }
        }
        if p.source_changes
            .iter()
            .any(|change| change["script"] == true && change["state"] != "removed")
        {
            p.warnings.push("Changed or added scripts are listed below, statically checked after download, and never executed by installation.".into());
        }

        if let Some(record) = active {
            for owned in &record.targets {
                if files::image(&files::read(profile, &owned.target)?)? != owned.image {
                    return Err(fail(
                        "LOCAL_MODIFIED",
                        "Local files differ from the owned version. Keep local changes or resolve them before updating.",
                    ));
                }
            }
            if let Some(previous) = record.source.as_ref() {
                if previous.repository != source.repository
                    || previous.package_root != source.package_root
                    || previous.kind != source.kind
                {
                    return Err(fail(
                        "SOURCE_CONFLICT",
                        "A managed update cannot silently change repository, component or kind.",
                    ));
                }
                if request.action == "update"
                    && previous.commit == source.commit
                    && previous.fingerprint == source.fingerprint
                {
                    p.blockers
                        .push("The inspected version is already installed.".into());
                }
            }
        }
        if source.kind == "mcp" {
            let draft=request.mcp.as_ref().ok_or_else(||fail("CONFIG_REQUIRED","Review and provide an MCP transport declaration before previewing its configuration."))?;
            let target = if request.scope == "project" {
                profile.project_root.join(".codex/config.toml")
            } else {
                profile.codex_home.join("config.toml")
            };
            let tree = files::read(profile, &target)?;
            let value = config::mcp_value(draft)?;
            if draft.auth_required {
                p.requirements.push("AUTH_REQUIRED: this MCP needs an external account or credentials; configuration does not authenticate it.".into());
            }
            let previous = config::entry(&tree, "toml", "mcp_servers", &draft.name)?;
            if previous.is_some() && active.is_none() {
                return Err(fail(
                    "NAME_CONFLICT",
                    "An MCP with this name already exists and is not owned by this installation.",
                ));
            }
            if let Some(record) = active {
                if !record.patches.iter().any(|p| {
                    p.target == target && p.patch.key == draft.name && p.patch.value == previous
                }) {
                    return Err(fail(
                        "CONFIG_CONFLICT",
                        "The owned MCP configuration was modified or the server name changed.",
                    ));
                }
            }
            let patch = config::new_patch(&tree, "toml", "mcp_servers", &draft.name, Some(value))?;
            p.changes
                .push(change(profile, target, "configuration", Some(patch))?);
            p.warnings.push("MCP is configured disabled. Dependencies and authorization remain separate; installation does not connect to a server.".into());
        } else {
            let folder = files::identifier(request.alias.as_deref().unwrap_or(&source.name))?;
            if source.kind == "plugin" && folder != source.name {
                return Err(fail(
                    "NAME_CONFLICT",
                    "Plugin identity must match its reviewed manifest. Choose a different project scope instead of rewriting the package.",
                ));
            }
            let root = if source.kind == "skill" {
                if request.scope == "project" {
                    profile.project_root.join(".agents/skills")
                } else {
                    profile.user_home.join(".agents/skills")
                }
            } else {
                if request.scope == "project" {
                    profile.project_root.join("plugins")
                } else {
                    profile.user_home.join("plugins")
                }
            };
            let target = if let Some(record) = active {
                record.target.clone()
            } else {
                root.join(&folder)
            };
            let c = change(profile, target.clone(), "package", None)?;
            if c.before.kind != "absent" && active.is_none() {
                return Err(fail(
                    "NAME_CONFLICT",
                    "The destination already exists and is not owned by SkillVerse. Keep it, choose a directory alias, or use another scope.",
                ));
            }
            p.changes.push(c);
            if source.kind == "plugin" {
                let cache = profile
                    .codex_home
                    .join("plugins")
                    .join("cache")
                    .join("skillverse-local")
                    .join(&source.name)
                    .join("local");
                let c = change(profile, cache, "package-cache", None)?;
                if c.before.kind != "absent" && active.is_none() {
                    return Err(fail(
                        "NAME_CONFLICT",
                        "An installed plugin cache already exists and is not managed by this record.",
                    ));
                }
                p.changes.push(c);
                let market = if request.scope == "project" {
                    profile
                        .project_root
                        .join(".agents/plugins/marketplace.json")
                } else {
                    profile.user_home.join(".agents/plugins/marketplace.json")
                };
                let tree = files::read(profile, &market)?;
                let value = json!({"name":source.name,"source":{"source":"local","path":format!("./plugins/{}",source.name)},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"});
                let previous =
                    config::entry(&tree, "marketplace", "skillverse-local", &source.name)?;
                if previous.is_some() && active.is_none() {
                    return Err(fail(
                        "MARKETPLACE_CONFLICT",
                        "A same-name marketplace entry is not owned by this operation.",
                    ));
                }
                if let Some(record) = active {
                    if !record
                        .patches
                        .iter()
                        .any(|p| p.target == market && p.patch.value == previous)
                    {
                        return Err(fail(
                            "MARKETPLACE_CONFLICT",
                            "The managed marketplace entry changed.",
                        ));
                    }
                }
                let patch = config::new_patch(
                    &tree,
                    "marketplace",
                    "skillverse-local",
                    &source.name,
                    Some(value),
                )?;
                p.changes
                    .push(change(profile, market, "configuration", Some(patch))?);
                let target_config = profile.codex_home.join("config.toml");
                let tree = files::read(profile, &target_config)?;
                let key = format!("{}@skillverse-local", source.name);
                let previous = config::entry(&tree, "toml", "plugins", &key)?;
                let enabled = previous
                    .as_ref()
                    .and_then(|p| p["enabled"].as_bool())
                    .unwrap_or(false);
                if previous.is_some() && active.is_none() {
                    return Err(fail(
                        "CONFIG_CONFLICT",
                        "A same-name plugin policy already exists and is not managed.",
                    ));
                }
                let value = previous.clone().unwrap_or_else(|| json!({"enabled":false}));
                let patch = config::new_patch(&tree, "toml", "plugins", &key, Some(value))?;
                if let Some(record) = active {
                    if !record
                        .patches
                        .iter()
                        .any(|p| p.target == target_config && p.patch.value == previous)
                    {
                        return Err(fail(
                            "CONFIG_CONFLICT",
                            "The managed plugin policy was modified.",
                        ));
                    }
                }
                p.changes.push(change(
                    profile,
                    target_config,
                    "configuration",
                    Some(patch),
                )?);
                p.warnings.push(if enabled{"The existing enabled policy is preserved; this update does not launch any plugin code."}else{"Plugin source, cache and marketplace registration will be installed with enabled=false. Codex recognition and runtime verification are not inferred."}.into());
            }
        }
        p.source = Some(source);
    } else {
        return Err(fail("ACTION", "Unsupported installation action."));
    }
    if p.changes.is_empty() {
        p.blockers.push("No reviewed changes are available.".into());
    }
    for c in &p.changes {
        files::space(&c.target, files::MAX_TOTAL * 4)?;
    }
    p.warnings.push("No third-party scripts, hooks or MCP processes will run. Failure recovery uses encrypted verified backups and an explicit operation journal.".into());
    p.can_execute = p.blockers.is_empty();
    finalize(p)
}
async fn rollback_plan(db: &Database, mut p: Plan) -> Result<Plan> {
    let id = p
        .rollback_operation
        .as_deref()
        .ok_or_else(|| fail("ROLLBACK", "Choose the operation to roll back."))?;
    let previous = db.install_operation(id).await?;
    if previous.plan.profile.id != p.profile.id
        || previous.plan.entity_id != p.entity_id
        || previous.state != "COMPLETED"
    {
        return Err(fail(
            "ROLLBACK",
            "Only a completed operation in this profile/capability can be reverted through a new plan.",
        ));
    }
    let current = db
        .install_records(&p.profile.id)
        .await?
        .into_iter()
        .find(|record| {
            Some(&record.id) == previous.plan.managed_id.as_ref()
                || record.current_operation_id == id
        })
        .ok_or_else(|| fail("ROLLBACK", "The managed ownership record is unavailable."))?;
    if current.current_operation_id != id {
        return Err(fail(
            "ROLLBACK_CONFLICT",
            "A later operation owns this installation. Roll back the most recent operation first.",
        ));
    }
    p.kind = previous.plan.kind.clone();
    p.source = previous.plan.source.clone();
    p.managed_id = Some(current.id.clone());
    p.prior_record = Some(current);
    let after = previous.recovery["after"]
        .as_array()
        .ok_or_else(|| fail("ROLLBACK", "Verified recovery metadata is unavailable."))?;
    for (index, old) in previous.plan.changes.iter().enumerate() {
        let tree = files::read(&p.profile, &old.target)?;
        let before = files::image(&tree)?;
        let snapshot = p
            .profile
            .work_root
            .join(id)
            .join(format!("before-{index}.sealed"));
        let original = super::sealed::load(&snapshot)?;
        let expected: Image =
            serde_json::from_value(after.get(index).cloned().unwrap_or(Value::Null))
                .map_err(|_| fail("ROLLBACK", "Recovery inventory is invalid."))?;
        let patch = if before == expected {
            None
        } else if let Some(patch) = old.patch.as_ref() {
            let written = config::entry(
                &super::sealed::load(
                    &p.profile
                        .work_root
                        .join(id)
                        .join(format!("after-{index}.sealed")),
                )?,
                &patch.format,
                &patch.section,
                &patch.key,
            )?;
            if config::entry(&tree, &patch.format, &patch.section, &patch.key)? != written {
                return Err(fail(
                    "ROLLBACK_CONFLICT",
                    "A managed configuration entry has later edits.",
                ));
            }
            let old_value = config::entry(&original, &patch.format, &patch.section, &patch.key)?;
            Some(config::new_patch(
                &tree,
                &patch.format,
                &patch.section,
                &patch.key,
                old_value,
            )?)
        } else {
            let expected: Image =
                serde_json::from_value(after.get(index).cloned().unwrap_or(Value::Null))
                    .map_err(|_| fail("ROLLBACK", "Recovery inventory is invalid."))?;
            if before != expected {
                return Err(fail(
                    "LOCAL_MODIFIED",
                    "Rollback would overwrite modified/unowned files.",
                ));
            }
            None
        };
        p.changes.push(Change {
            target: old.target.clone(),
            staging_path: PathBuf::new(),
            holding_path: PathBuf::new(),
            role: "restore".into(),
            before,
            patch,
            restore_from: Some(format!("{id}/before-{index}.sealed")),
        });
    }
    p.warnings.push("Restores the selected operation's owned changes while preserving unrelated configuration entries. User edits block rollback.".into());
    p.can_execute = true;
    Ok(p)
}

fn finalize(mut p: Plan) -> Result<Plan> {
    files::space(&p.profile.work_root, files::MAX_TOTAL * 8)?;
    for (index, change) in p.changes.iter_mut().enumerate() {
        change.staging_path =
            files::scratch(&p.profile, &change.target, &p.operation_id, index, "new")?;
        change.holding_path =
            files::scratch(&p.profile, &change.target, &p.operation_id, index, "old")?;
    }
    Ok(p)
}
