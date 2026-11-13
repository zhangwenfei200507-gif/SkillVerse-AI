use super::{
    analysis::package_fingerprint,
    git_blob,
    types::{ApiError, now, safe_path},
};
use crate::db::Database;
use serde_json::{Value, json};
#[cfg(test)]
use std::path::PathBuf;
use std::{collections::BTreeMap, path::Path};

pub async fn plan(db: &Database, id: &str) -> Result<Value, ApiError> {
    let capability = db
        .get_capability(id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::new("NOT_FOUND", "Capability was not found."))?;
    let intelligence: Option<String> = sqlx::query_scalar(
        "SELECT payload_json FROM github_capability_intelligence WHERE entity_id=?",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|_| ApiError::database())?;
    let data: Value = intelligence
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null);
    let mut blockers: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let availability: Option<String> = sqlx::query_scalar("SELECT r.availability FROM github_capability_sources s JOIN github_repositories r ON r.id=s.repository_id WHERE s.entity_id=?").bind(id).fetch_optional(&db.pool).await.map_err(|_|ApiError::database())?;
    if availability.as_deref().is_some_and(|s| s != "AVAILABLE") {
        blockers.push("The repository is archived, disabled, or unavailable; review its source before installation.".into());
    }
    if data["sourceState"].as_str().is_some_and(|s| s != "PRESENT") {
        blockers
            .push("The source is missing or changed and needs a fresh manifest inspection.".into());
    }
    if data.is_null() {
        blockers
            .push("Run a read-only manifest inspection before planning installation.".to_string());
    }
    if data["compatibility"] == "INCOMPATIBLE" || data["compatibility"] == "UNVERIFIED" {
        blockers.push("Compatibility has not been established.".into());
    }
    if data["reviewState"] == "NEEDS_REVIEW" {
        blockers.push("Security findings require review before a future installation.".into());
    }
    if data["packageFingerprint"].is_null() {
        blockers.push("The complete package inventory could not be verified.".into());
    }
    let home = crate::codex_scanner::resolve_codex_home()
        .ok_or_else(|| ApiError::new("CODEX_HOME", "Codex home could not be resolved."))?;
    let declared_name = data["manifestName"].as_str().unwrap_or(&capability.name);
    let valid_name = safe_path(declared_name).ok().filter(|s| {
        !s.contains('/')
            && s.len() <= 100
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && ![
                "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
                "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
                "lpt9",
            ]
            .contains(&s.to_ascii_lowercase().as_str())
    });
    if valid_name.is_none() {
        blockers.push("The manifest name is not a safe single-directory identifier.".into());
    }
    let name = valid_name.unwrap_or_else(|| "unverified-capability".into());
    let kind = capability.kind.as_str();
    let target = if kind == "mcp" {
        home.join("config.toml")
    } else {
        home.join(if kind == "plugin" {
            "plugins"
        } else {
            "skills"
        })
        .join(&name)
    };
    if !contained_target(&home, &target) {
        blockers.push("A symlink, reparse point, or filesystem path would escape the intended Codex directory.".into());
    }
    let exists = target.exists();
    let local_fingerprint = if exists && target.is_dir() && contained_target(&home, &target) {
        local_package_fingerprint(&target)
    } else {
        None
    };
    let remote_fingerprint = data["packageFingerprint"].as_str();
    let local_state = if kind == "mcp" {
        "CONFIGURATION_NOT_CONNECTED"
    } else if !exists {
        "NOT_FOUND_AT_TARGET"
    } else if local_fingerprint.as_deref() == remote_fingerprint && remote_fingerprint.is_some() {
        "EXACT_PACKAGE_PRESENT"
    } else {
        "LOCAL_MODIFIED_OR_VERSION_UNKNOWN"
    };
    if exists && local_state != "EXACT_PACKAGE_PRESENT" {
        warnings.push("The proposed target already exists. A future installer must preserve local changes and obtain approval.".into());
    }
    let package_root = data["packageRoot"].as_str().unwrap_or("");
    let prefix = if package_root.is_empty() {
        String::new()
    } else {
        format!("{package_root}/")
    };
    let mut writes = Vec::new();
    if kind == "mcp" {
        writes.push(json!({"sourcePath":data["packageRoot"],"targetPath":target.to_string_lossy(),"conflict":exists,"operation":"reviewed TOML merge; never copy JSON over config.toml"}));
    } else if let Some(files) = data["packageFiles"].as_array() {
        for file in files.iter().filter_map(Value::as_str) {
            let Some(relative) = file.strip_prefix(&prefix) else {
                blockers.push("Inventory contains a path outside the reviewed package.".into());
                continue;
            };
            match safe_path(relative) {
                Ok(relative) => {
                    let destination = target.join(&relative);
                    if contained_target(&home, &destination) {
                        writes.push(json!({"sourcePath":file,"targetPath":destination.to_string_lossy(),"conflict":destination.exists()}));
                    } else {
                        blockers.push(
                            "An inventory target would escape the installation directory.".into(),
                        );
                    }
                }
                Err(_) => blockers.push("Inventory contains an unsafe path.".into()),
            }
        }
    }
    let dependencies = data["dependencies"].as_array().cloned().unwrap_or_default();
    let mut runtimes = Vec::new();
    for (marker, command) in [
        ("package.json", "node"),
        ("requirements.txt", "python"),
        ("pyproject.toml", "python"),
        ("Cargo.toml", "cargo"),
        (".sh", "bash"),
    ] {
        if dependencies
            .iter()
            .filter_map(Value::as_str)
            .any(|p| p.ends_with(marker))
        {
            runtimes.push(json!({"command":command,"foundOnPath":command_on_path(command)}));
        }
    }
    for command in dependencies
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|s| s.strip_prefix("command:"))
    {
        runtimes.push(json!({"command":command,"foundOnPath":command_on_path(command)}));
    }
    let linked_local: Vec<String> = sqlx::query_scalar(
        "SELECT local_entity_id FROM github_local_links WHERE remote_entity_id=?",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .map_err(|_| ApiError::database())?;
    let configuration_required = data["compatibility"] == "REQUIRES_CONFIGURATION";
    if configuration_required {
        warnings.push("External accounts, environment variables, transport configuration, or local runtimes require setup.".into());
    }
    blockers.sort();
    blockers.dedup();
    Ok(
        json!({"entityId":id,"dryRun":true,"canExecute":false,"phase":"Phase 5 preview only; execution requires Phase 6 and explicit authorization","sourceRepository":format!("{}/{}",capability.source_owner.as_deref().unwrap_or(""),capability.source_repo.as_deref().unwrap_or("")),"sourcePath":serde_json::from_str::<Value>(&capability.metadata_json).ok().and_then(|v|v["discovery"]["sourcePath"].as_str().map(str::to_string)),"commit":data["latestCommit"],"kind":kind,"targetDirectory":target.to_string_lossy(),"writePreview":writes,"configurationRequired":configuration_required,"requirements":data["requirements"],"dependencies":runtimes,"localState":local_state,"matchingLocalEntityIds":linked_local,"localVersion":"No version is inferred from a matching display name","remoteVersion":data["remoteVersion"],"risks":data["risks"],"blockers":blockers,"warnings":warnings,"rollback":{"snapshotCreated":false,"strategy":"Phase 6 must create and verify a target backup before any writes; preserve local modifications"},"generatedAt":now()}),
    )
}
pub fn contained_target(root: &Path, target: &Path) -> bool {
    if !target.starts_with(root) {
        return false;
    }
    let Ok(relative) = target.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return false;
        }
        current.push(component);
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return false;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
        }
    }
    true
}
fn command_on_path(command: &str) -> bool {
    if command.is_empty()
        || command.len() > 100
        || !command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return false;
    }

    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|dir| !dir.to_string_lossy().starts_with("\\\\"))
                .any(|dir| {
                    [".exe", ".cmd", ""]
                        .iter()
                        .any(|ext| dir.join(format!("{command}{ext}")).is_file())
                })
        })
        .unwrap_or(false)
}
pub fn local_package_fingerprint(root: &Path) -> Option<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut tree = BTreeMap::new();
    let mut bytes_total = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if !contained_target(root, &path) {
                return None;
            }
            if metadata.is_dir() {
                if entry.file_name() == ".git" {
                    continue;
                }
                pending.push(path);
                if pending.len() > 500 {
                    return None;
                }
            } else if metadata.is_file() {
                if tree.len() >= 500 {
                    return None;
                }
                bytes_total = bytes_total.checked_add(metadata.len() as usize)?;
                if bytes_total > 8 * 1024 * 1024 {
                    return None;
                }
                let data = std::fs::read(&path).ok()?;
                let relative = path
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                tree.insert(
                    relative,
                    (
                        git_blob(&data),
                        if is_executable(&metadata) {
                            "100755"
                        } else {
                            "100644"
                        }
                        .into(),
                    ),
                );
            } else {
                return None;
            }
        }
    }
    package_fingerprint("", &tree)
}
pub async fn local_inventory(db: &Database) -> Result<Vec<(String, String)>, ApiError> {
    let Some(home) = crate::codex_scanner::resolve_codex_home() else {
        return Ok(vec![]);
    };
    let ids: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,kind FROM entities WHERE source_type='local' AND kind IN ('skill','plugin')",
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|_| ApiError::database())?;
    tokio::task::spawn_blocking(move || inventory_under(&home, &ids))
        .await
        .map_err(|_| ApiError::new("LOCAL_READ", "Local package comparison was interrupted."))
}
fn inventory_under(home: &Path, ids: &[(String, String)]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut pending = vec![(home.join("skills"), 0), (home.join("plugins"), 0)];
    let mut visited = 0;
    while let Some((directory, depth)) = pending.pop() {
        visited += 1;
        if visited > 2000 {
            break;
        }
        if depth > 8 || !contained_target(home, &directory) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for kind in ["skill", "plugin"] {
            let candidate = if kind == "skill" {
                directory.join("SKILL.md").canonicalize().ok()
            } else {
                Some(directory.clone())
            };
            if let Some(path) = candidate {
                let id = crate::codex_scanner::stable_id(kind, &path);
                if ids.iter().any(|(local, k)| *local == id && k == kind) {
                    if let Some(fp) = local_package_fingerprint(&directory) {
                        found.push((id, fp));
                    }
                }
            }
        }
        for entry in entries.take(500).flatten() {
            if entry.file_type().is_ok_and(|m| m.is_dir()) {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    found
}
pub async fn link_fingerprints(
    db: &Database,
    remote: &str,
    a: &super::types::Assessment,
    inventory: &[(String, String)],
) -> Result<(), ApiError> {
    // Links describe this comparison, not an installation state inferred by name.
    sqlx::query("DELETE FROM github_local_links WHERE remote_entity_id=?")
        .bind(remote)
        .execute(&db.pool)
        .await
        .map_err(|_| ApiError::database())?;
    if let Some(expected) = a.package_fingerprint.as_ref() {
        for (local, fp) in inventory {
            if fp == expected {
                db.gh_link_local(local, remote, expected).await?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
pub(super) async fn link_local_under(
    db: &Database,
    remote: &str,
    a: &super::types::Assessment,
    home: &Path,
) -> Result<(), ApiError> {
    let ids: Vec<(String, String)> =
        sqlx::query_as("SELECT id,kind FROM entities WHERE source_type='local'")
            .fetch_all(&db.pool)
            .await
            .map_err(|_| ApiError::database())?;
    link_fingerprints(db, remote, a, &inventory_under(home, &ids)).await
}
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traversal_never_produces_a_write_target() {
        let root = PathBuf::from("D:/test-codex");
        assert!(!contained_target(&root, &root.join("../other")));
        assert!(!contained_target(&root, &PathBuf::from("D:/other/skill")));
    }
}
