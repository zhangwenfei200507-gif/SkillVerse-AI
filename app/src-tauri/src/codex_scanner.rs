use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::db::InstallationStatus;

const SKILL_LIMIT: usize = 64 * 1024;
const PLUGIN_MANIFEST_LIMIT: usize = 256 * 1024;
const DIRECTORY_ENTRY_LIMIT: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedEntity {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    pub status: InstallationStatus,
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: String,
    pub enabled: Option<bool>,
    pub valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpFinding {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub transport: String,
    pub enabled: bool,
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFinding {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub enabled: Option<bool>,
    pub valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexScanReport {
    pub codex_home_found: bool,
    pub cli_found_on_path: bool,
    pub scanned_at_unix_ms: u64,
    pub skills: Vec<SkillFinding>,
    pub mcp_servers: Vec<McpFinding>,
    pub plugins: Vec<PluginFinding>,
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<crate::capability_identity::IdentityScan>,
    #[serde(skip)]
    pub(crate) entities: Vec<ScannedEntity>,
}

pub(crate) fn scan_with_paths(
    user_home: Option<PathBuf>,
    codex_home: Option<PathBuf>,
    workspace: Option<PathBuf>,
) -> CodexScanReport {
    let mut report = CodexScanReport {
        codex_home_found: codex_home.as_ref().is_some_and(|path| path.is_dir()),
        cli_found_on_path: cli_exists_on_path(),
        scanned_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        skills: Vec::new(),
        mcp_servers: Vec::new(),
        plugins: Vec::new(),
        warnings: Vec::new(),
        identity: None,
        entities: Vec::new(),
    };
    let mut seen_skill_files = HashSet::new();

    let config = codex_home.as_ref().and_then(|path| read_config(path));
    if let Some(home) = user_home {
        scan_skill_root(
            &home.join(".agents").join("skills"),
            "user",
            &mut seen_skill_files,
            &mut report,
            config.as_ref(),
        );
    }
    if let Some(codex_home) = codex_home.as_ref() {
        scan_skill_root(
            &codex_home.join("skills"),
            "codex-home",
            &mut seen_skill_files,
            &mut report,
            config.as_ref(),
        );
        scan_plugin_cache(
            &codex_home.join("plugins").join("cache"),
            &mut report,
            config.as_ref(),
        );
        scan_mcp_config(&codex_home.join("config.toml"), "user", true, &mut report);
    }
    if let Some(workspace) = workspace {
        scan_workspace_skills(
            &workspace,
            &mut seen_skill_files,
            &mut report,
            config.as_ref(),
        );
        scan_mcp_config(
            &workspace.join(".codex/config.toml"),
            "project",
            false,
            &mut report,
        );
    }

    report.skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.scope.cmp(&right.scope))
    });
    report
        .mcp_servers
        .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    report.plugins.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.version.cmp(&right.version))
    });
    report
}

pub(crate) fn resolve_codex_home() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    resolve_user_home().map(|home| home.join(".codex"))
}

fn resolve_user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )))
        })
}

pub(crate) fn cli_exists_on_path() -> bool {
    let Some(path_value) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_value).any(|directory| {
        ["codex.exe", "codex.cmd", "codex.bat", "codex"]
            .iter()
            .any(|name| directory.join(name).is_file())
    })
}

fn scan_workspace_skills(
    workspace: &Path,
    seen: &mut HashSet<PathBuf>,
    report: &mut CodexScanReport,
    config: Option<&toml::Value>,
) {
    let mut cursor = match fs::canonicalize(workspace) {
        Ok(path) => path,
        Err(_) => return,
    };
    let mut roots = Vec::new();
    let repo_root = find_repo_root(&cursor);
    loop {
        roots.push(cursor.join(".agents").join("skills"));
        if repo_root.as_ref().is_some_and(|root| *root == cursor) {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent.to_path_buf();
        if repo_root.is_none() {
            break;
        }
    }
    for root in roots {
        scan_skill_root(&root, "project", seen, report, config);
    }
}

fn find_repo_root(workspace: &Path) -> Option<PathBuf> {
    workspace
        .ancestors()
        .find(|path| path.join(".git").exists())
        .map(Path::to_path_buf)
}

fn scan_skill_root(
    root: &Path,
    scope: &str,
    seen: &mut HashSet<PathBuf>,
    report: &mut CodexScanReport,
    config: Option<&toml::Value>,
) {
    let Some(entries) = bounded_directories(root, scope, report) else {
        return;
    };
    for skill_dir in entries {
        read_skill(&skill_dir, scope, None, seen, report, config);
    }
}

fn read_skill(
    skill_dir: &Path,
    scope: &str,
    plugin: Option<&str>,
    seen: &mut HashSet<PathBuf>,
    report: &mut CodexScanReport,
    config: Option<&toml::Value>,
) {
    let manifest_path = skill_dir.join("SKILL.md");
    let Ok(metadata) = fs::symlink_metadata(&manifest_path) else {
        return;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return;
    }
    if metadata.len() > SKILL_LIMIT as u64 {
        report.warnings.push(format!(
            "A {scope} skill manifest exceeded the 64 KB scan limit."
        ));
        return;
    }
    let Ok(canonical_path) = fs::canonicalize(&manifest_path) else {
        return;
    };
    if !seen.insert(canonical_path.clone()) {
        return;
    }
    let raw = match fs::read_to_string(&manifest_path) {
        Ok(value) => value,
        Err(_) => {
            report
                .warnings
                .push(format!("A {scope} skill manifest could not be read."));
            return;
        }
    };
    let parsed = parse_skill_frontmatter(&raw);
    let name = parsed
        .as_ref()
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| {
            skill_dir
                .file_name()
                .and_then(|part| part.to_str())
                .unwrap_or("Unnamed skill")
                .chars()
                .take(120)
                .collect()
        });
    let description = parsed
        .as_ref()
        .map(|(_, description)| description.clone())
        .unwrap_or_else(|| "Skill manifest could not be validated.".into());
    let enabled = skill_enabled(&canonical_path, config);
    let id = stable_id("skill", &canonical_path);
    let valid = parsed.is_some();
    let mut evidence = json!({"scope": scope, "manifestValid": valid, "enabled": enabled, "runtimeInvoked": false,"manifestPath":manifest_path,"installationPath":skill_dir});
    if let Some(plugin_name) = plugin {
        evidence["plugin"] = Value::String(plugin_name.into());
    }
    report.entities.push(ScannedEntity {
        id: id.clone(),
        kind: "skill".into(),
        name: name.clone(),
        summary: description.clone(),
        status: if valid {
            InstallationStatus::Installed
        } else {
            InstallationStatus::Error
        },
        evidence,
    });
    report.skills.push(SkillFinding {
        id,
        name,
        description,
        scope: scope.into(),
        enabled,
        valid,
    });
}

fn parse_skill_frontmatter(raw: &str) -> Option<(String, String)> {
    let mut lines = raw.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = None;
    for line in lines.take(100) {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = normalize_frontmatter_value(value);
        }
        if let Some(value) = line.strip_prefix("description:") {
            description = normalize_frontmatter_value(value);
        }
    }
    let name = name?;
    let description = description?;
    if name.is_empty() || name.len() > 120 || description.is_empty() || description.len() > 1000 {
        return None;
    }
    Some((name, description))
}

fn normalize_frontmatter_value(raw: &str) -> Option<String> {
    let value = raw
        .trim()
        .trim_matches(|character| character == '"' || character == '\'')
        .trim()
        .to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn read_config(codex_home: &Path) -> Option<toml::Value> {
    let path = codex_home.join("config.toml");
    let meta = fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 1024 * 1024 {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

fn skill_enabled(path: &Path, config: Option<&toml::Value>) -> Option<bool> {
    config?
        .get("skills")?
        .get("config")?
        .as_array()?
        .iter()
        .find_map(|entry| {
            let configured_path = entry.get("path")?.as_str()?;
            let configured = fs::canonicalize(configured_path).ok()?;
            if configured == path {
                entry.get("enabled").and_then(toml::Value::as_bool)
            } else {
                None
            }
        })
        .or(Some(true))
}

fn scan_mcp_config(
    config_path: &Path,
    scope: &str,
    effective_scope: bool,
    report: &mut CodexScanReport,
) {
    let Ok(meta) = fs::symlink_metadata(config_path) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return;
    }
    if meta.len() > 1024 * 1024 {
        report.warnings.push(format!(
            "The {scope} Codex config exceeded the 1 MB scan limit."
        ));
        return;
    }
    let Ok(content) = fs::read_to_string(config_path) else {
        report
            .warnings
            .push(format!("The {scope} Codex config could not be read."));
        return;
    };
    let Ok(parsed) = toml::from_str::<toml::Value>(&content) else {
        report.warnings.push(format!(
            "The {scope} Codex config is invalid TOML; MCP entries were not inferred."
        ));
        return;
    };
    let Some(servers) = parsed.get("mcp_servers").and_then(toml::Value::as_table) else {
        return;
    };
    for (name, value) in servers.iter().take(DIRECTORY_ENTRY_LIMIT) {
        let Some(table) = value.as_table() else {
            continue;
        };
        let enabled = table
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        let transport = match (
            table.get("command").and_then(toml::Value::as_str),
            table.get("url").and_then(toml::Value::as_str),
        ) {
            (Some(_), None) => "stdio",
            (None, Some(_)) => "streamable-http",
            _ => "unknown",
        };
        let safe_name: String = name.chars().take(120).collect();
        let id = stable_id("mcp", &format!("{}|{}", config_path.display(), name));
        let configured = transport != "unknown";
        report.entities.push(ScannedEntity {
            id: id.clone(), kind: "mcp".into(), name: safe_name.clone(),
            summary: "MCP server entry read from Codex configuration; server was not launched or contacted.".into(),
            status: if configured { InstallationStatus::Installed } else { InstallationStatus::Error },
            evidence: json!({"scope": scope, "transport": transport, "enabled": enabled, "effectiveConfigScope": effective_scope, "runtimeChecked": false,"configPath":config_path,"entryName":name}),
        });
        report.mcp_servers.push(McpFinding {
            id,
            name: safe_name,
            scope: scope.into(),
            transport: transport.into(),
            enabled: enabled && effective_scope,
            verification: if effective_scope {
                "configured-not-connected".into()
            } else {
                "project-trust-not-verified".into()
            },
        });
    }
}

fn scan_plugin_cache(
    cache_root: &Path,
    report: &mut CodexScanReport,
    config: Option<&toml::Value>,
) {
    let Some(markets) = bounded_directories(cache_root, "plugin cache", report) else {
        return;
    };
    let mut count = 0usize;
    for market in markets {
        let Some(plugins) = bounded_directories(&market, "marketplace cache", report) else {
            continue;
        };
        for plugin in plugins {
            let Some(versions) = bounded_directories(&plugin, "plugin versions", report) else {
                continue;
            };
            for version in versions {
                count += 1;
                if count > DIRECTORY_ENTRY_LIMIT {
                    report
                        .warnings
                        .push("Plugin scan reached the 500 entry limit.".into());
                    return;
                }
                read_plugin(&version, report, config);
            }
        }
    }
}

// Fail incomplete scans explicitly; absence is never inferred from enumeration errors.
fn bounded_directories(
    root: &Path,
    scope: &str,
    report: &mut CodexScanReport,
) -> Option<Vec<PathBuf>> {
    let meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            report
                .warnings
                .push(format!("Could not inspect the {scope} directory."));
            return None;
        }
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        report
            .warnings
            .push(format!("Could not safely enumerate the {scope} directory."));
        return None;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            report
                .warnings
                .push(format!("Could not read the {scope} directory."));
            return None;
        }
    };
    let mut result = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= DIRECTORY_ENTRY_LIMIT {
            report
                .warnings
                .push(format!("The {scope} scan reached its entry limit."));
            return None;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report
                    .warnings
                    .push(format!("Could not read a {scope} entry."));
                return None;
            }
        };
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(_) => {
                report
                    .warnings
                    .push(format!("Could not inspect a {scope} entry."));
                return None;
            }
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            result.push(entry.path());
        }
    }
    Some(result)
}

fn read_plugin(plugin_path: &Path, report: &mut CodexScanReport, config: Option<&toml::Value>) {
    let primary = plugin_path.join("plugin.json");
    let codex_manifest = plugin_path.join(".codex-plugin").join("plugin.json");
    let manifest = if primary.is_file() {
        primary
    } else {
        codex_manifest
    };
    let valid =
        fs::metadata(&manifest).is_ok_and(|meta| meta.len() <= PLUGIN_MANIFEST_LIMIT as u64);
    let parsed = if valid {
        fs::read_to_string(&manifest)
            .ok()
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
    } else {
        None
    };
    let name = parsed
        .as_ref()
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .map(|name| name.chars().take(120).collect::<String>())
        .unwrap_or_else(|| {
            plugin_path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or("Unknown plugin")
                .chars()
                .take(120)
                .collect()
        });
    let version = parsed
        .as_ref()
        .and_then(|value| value.get("version"))
        .and_then(Value::as_str)
        .map(|value| value.chars().take(80).collect());
    let id = stable_id("plugin", plugin_path);
    report.entities.push(ScannedEntity {
        id: id.clone(), kind: "plugin".into(), name: name.clone(), summary: "Plugin package found in the Codex plugin cache; hooks and tools were not executed.".into(),
        status: if parsed.is_some() { InstallationStatus::Installed } else { InstallationStatus::Error },
        evidence: json!({"scope": "user", "manifestValid": parsed.is_some(), "enabled": plugin_enabled(&name, plugin_path, config), "runtimeChecked": false,"installationPath":plugin_path,"manifestPath":manifest,"packageVersion":version}),
    });
    report.plugins.push(PluginFinding {
        id,
        name: name.clone(),
        version,
        enabled: plugin_enabled(&name, plugin_path, config),
        valid: parsed.is_some(),
    });

    let mut seen = HashSet::new();
    scan_skill_root(
        &plugin_path.join("skills"),
        "plugin",
        &mut seen,
        report,
        config,
    );
}

fn plugin_enabled(name: &str, plugin_path: &Path, config: Option<&toml::Value>) -> Option<bool> {
    let plugins = config?.get("plugins")?.as_table()?;
    let marketplace = plugin_path.parent()?.parent()?.file_name()?.to_str()?;
    plugins
        .get(&format!("{name}@{marketplace}"))
        .or_else(|| plugins.get(name))
        .and_then(|v| v.get("enabled"))
        .and_then(toml::Value::as_bool)
}

pub(crate) fn stable_id(kind: &str, path_or_key: impl AsRef<Path>) -> String {
    let source = path_or_key.as_ref().to_string_lossy().to_lowercase();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    format!("{kind}:local:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skillverse-scan-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn reads_valid_user_skills_and_mcp_metadata_without_returning_secret_values() {
        let root = fixture_root();
        let codex_home = root.join("codex-home");
        let skills = root
            .join("user")
            .join(".agents")
            .join("skills")
            .join("hello");
        fs::create_dir_all(&skills).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(
            skills.join("SKILL.md"),
            "---\nname: hello\ndescription: Safe skill metadata\n---\nTOP SECRET BODY\n",
        )
        .unwrap();
        fs::write(codex_home.join("config.toml"), "[mcp_servers.docs]\nurl = 'https://example.test/mcp'\nenabled = false\nhttp_headers = { Authorization = 'do-not-leak' }\n").unwrap();

        let report = scan_with_paths(Some(root.join("user")), Some(codex_home), None);
        assert_eq!(report.skills.len(), 1);
        assert!(report.skills[0].valid);
        assert_eq!(report.mcp_servers.len(), 1);
        assert!(!report.mcp_servers[0].enabled);
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("do-not-leak"));
        assert!(!serialized.contains("TOP SECRET BODY"));
        fs::remove_dir_all(root).unwrap();
    }
}
