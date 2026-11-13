mod store;
#[cfg(test)]
mod tests;
use crate::{
    codex_scanner::{self, CodexScanReport},
    github::types::digest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
pub(crate) use store::fingerprint as database_fingerprint;
pub(crate) use store::fingerprint_conn as database_fingerprint_on;
pub use store::{BackupRef, ReconciliationPlan};
pub(crate) use store::{schema_ready, sync_in};
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanContext {
    pub user_home: PathBuf,
    pub codex_home: PathBuf,
    pub project_root: Option<PathBuf>,
    pub user_key: String,
    pub project_key: Option<String>,
}
impl ScanContext {
    pub fn selected(
        user_home: PathBuf,
        codex_home: PathBuf,
        project_root: Option<PathBuf>,
    ) -> Self {
        let user_key = format!("user:{}", hash(&normalized(&user_home)));
        let project_key = project_root
            .as_ref()
            .map(|p| format!("project:{}", hash(&normalized(p))));
        Self {
            user_home,
            codex_home,
            project_root,
            user_key,
            project_key,
        }
    }
    pub fn scopes(&self) -> Vec<String> {
        std::iter::once(self.user_key.clone())
            .chain(self.project_key.clone())
            .collect()
    }
    pub fn validate(&self) -> Result<()> {
        for p in [&self.user_home, &self.codex_home]
            .into_iter()
            .chain(self.project_root.iter())
        {
            if !p.is_absolute() || !p.is_dir() || !ordinary(p) {
                return Err("An explicit scan root is unavailable or contains a reparse point. No missing installations may be inferred.".into());
            }
        }
        if self.user_key.is_empty()
            || self.user_key.len() > 160
            || self.project_root.is_some() != self.project_key.is_some()
        {
            return Err(
                "An explicit stable project scope key is required with the selected project root."
                    .into(),
            );
        }
        if let Some(root) = self.project_root.as_ref() {
            if let Some(repo) = root.ancestors().find(|p| p.join(".git").exists()) {
                if repo != root {
                    return Err("Select the repository root explicitly; scanning above the selected project root is not allowed.".into());
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub entity_id: String,
    pub physical_id: String,
    pub logical_id: String,
    pub kind: String,
    pub machine_name: String,
    pub origin_key: String,
    pub authority: Value,
    pub component_key: String,
    pub scope_key: String,
    pub location: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub package_version: Option<String>,
    pub cache_version: Option<String>,
    pub manifest_hash: String,
    pub present: bool,
    pub status: String,
    pub confidence: String,
    pub evidence: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityScan {
    pub context: ScanContext,
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub history: Vec<Observation>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub duplicate_manifests: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}
impl IdentityScan {
    pub fn fingerprint(&self) -> String {
        let mut rows = self.observations.clone();
        rows.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        hash(&json!({"context":self.context,"observations":rows}).to_string())
    }
}
pub fn normalized(p: &Path) -> String {
    let raw = p.to_string_lossy().replace('\\', "/");
    let raw = raw
        .strip_prefix("//?/")
        .unwrap_or(&raw)
        .trim_end_matches('/');
    if cfg!(windows) {
        raw.to_lowercase()
    } else {
        raw.to_owned()
    }
}
pub fn hash(s: &str) -> String {
    digest(s.as_bytes())
}
fn ordinary(p: &Path) -> bool {
    p.ancestors().all(|a| match fs::symlink_metadata(a) {
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
            !m.file_type().is_symlink()
        }
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    })
}
fn read(p: &Path, limit: u64) -> Result<Vec<u8>> {
    if !ordinary(p) {
        return Err("A metadata path is a link/reparse point.".into());
    }
    let m = fs::metadata(p).map_err(|_| "A required metadata file cannot be read.")?;
    if !m.is_file() || m.len() > limit {
        return Err("A metadata file exceeds its bounds.".into());
    }
    fs::read(p).map_err(|_| "A required metadata file cannot be read.".into())
}
fn json_file(p: &Path) -> Result<(Value, String)> {
    let bytes = read(p, 1024 * 1024)?;
    let v = serde_json::from_slice(&bytes).map_err(|_| "Invalid package/catalog JSON.")?;
    Ok((v, digest(&bytes)))
}
fn repository(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let s = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("git@github.com:"))
        .or_else(|| s.strip_prefix("ssh://git@github.com/"))?;
    let s = s.trim_end_matches('/').trim_end_matches(".git");
    crate::github::types::normalize_repository(s).ok()
}
fn core_manifest(v: &Value) -> Value {
    json!({"machine":v["name"].as_str().unwrap_or("").to_lowercase(),"repository":repository(&v["repository"]),"author":v["author"]["url"].as_str().filter(|s|s.starts_with("https://")&&!s.contains(['?','@','#']))})
}
#[derive(Clone)]
struct Registration {
    alias: String,
    catalog: String,
    component: String,
    machine: String,
    source_core: Value,
    source_hash: String,
    source_version: Option<String>,
    origin: Value,
    source_root: PathBuf,
    catalog_path: PathBuf,
}
fn catalog_registrations(
    alias: &str,
    root: &Path,
    catalog_path: &Path,
    warnings: &mut Vec<String>,
) -> Vec<Registration> {
    let mut result = Vec::new();
    if !root.is_absolute() || !root.is_dir() || !ordinary(root) {
        warnings.push(format!(
            "Marketplace {alias} has an unavailable/untrusted root."
        ));
        return result;
    }
    let (catalog, _) = match json_file(catalog_path) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(e);
            return result;
        }
    };
    let Some(catalog_name) = catalog["name"].as_str() else {
        warnings.push("A marketplace lacks its machine identifier.".into());
        return result;
    };
    let Some(entries) = catalog["plugins"].as_array() else {
        warnings.push("Invalid marketplace plugin array.".into());
        return result;
    };
    let mut names = std::collections::HashSet::new();
    if entries
        .iter()
        .filter_map(|e| e["name"].as_str())
        .any(|n| !names.insert(n))
    {
        warnings.push(format!(
            "Marketplace {alias} has ambiguous duplicate component identifiers."
        ));
        return result;
    }
    for entry in entries.iter().take(500) {
        if entry["source"]["source"] != "local" {
            continue;
        }
        let Some(relative) = entry["source"]["path"]
            .as_str()
            .and_then(|s| s.strip_prefix("./"))
            .and_then(|s| crate::github::types::safe_path(s).ok())
        else {
            continue;
        };
        let package = root.join(&relative);
        if !ordinary(&package) {
            warnings.push("A marketplace component crosses a link/reparse point.".into());
            continue;
        }
        let manifest = if package.join("plugin.json").is_file() {
            package.join("plugin.json")
        } else {
            package.join(".codex-plugin/plugin.json")
        };
        let (value, source_hash) = match json_file(&manifest) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(machine) = value["name"].as_str() else {
            continue;
        };
        if entry["name"] != machine {
            warnings.push(format!(
                "Marketplace {alias} component and manifest identifiers disagree."
            ));
            continue;
        }
        let core = core_manifest(&value);
        let origin = json!({"type":"registered-local-marketplace","registration":alias,"catalog":catalog_name,"repository":core["repository"],"publisher":core["author"]});
        result.push(Registration {
            alias: alias.into(),
            catalog: catalog_name.into(),
            component: relative,
            machine: machine.into(),
            source_core: core,
            source_hash,
            source_version: value["version"].as_str().map(str::to_owned),
            origin,
            source_root: root.into(),
            catalog_path: catalog_path.into(),
        });
    }
    result
}
fn registrations(ctx: &ScanContext, warnings: &mut Vec<String>) -> Vec<Registration> {
    let mut selected = BTreeMap::<String, (PathBuf, PathBuf)>::new();
    // Default personal/repository catalogs resolve source paths against the home/repository root.
    for root in [Some(ctx.user_home.clone()), ctx.project_root.clone()]
        .into_iter()
        .flatten()
    {
        let path = root.join(".agents/plugins/marketplace.json");
        if !path.exists() {
            continue;
        }
        if let Ok((catalog, _)) = json_file(&path) {
            if let Some(alias) = catalog["name"].as_str() {
                if selected.contains_key(alias) {
                    warnings.push(format!("Ambiguous default marketplace namespace {alias}."));
                } else {
                    selected.insert(alias.into(), (root, path));
                }
            }
        }
    }
    // Project configuration overrides user configuration for the same registered namespace.
    for path in [
        Some(ctx.codex_home.join("config.toml")),
        ctx.project_root
            .as_ref()
            .map(|p| p.join(".codex/config.toml")),
    ]
    .into_iter()
    .flatten()
    {
        if !path.exists() {
            continue;
        }
        let parsed = read(&path, 1024 * 1024)
            .and_then(|b| {
                String::from_utf8(b).map_err(|_| "Invalid configuration encoding.".into())
            })
            .and_then(|s| {
                toml::from_str::<toml::Value>(&s).map_err(|_| "Invalid configuration TOML.".into())
            });
        let config = match parsed {
            Ok(v) => v,
            Err(e) => {
                warnings.push(e);
                continue;
            }
        };
        let Some(markets) = config.get("marketplaces").and_then(toml::Value::as_table) else {
            continue;
        };
        for (alias, entry) in markets.iter().take(100) {
            if entry.get("source_type").and_then(toml::Value::as_str) != Some("local") {
                continue;
            }
            let Some(root) = entry
                .get("source")
                .and_then(toml::Value::as_str)
                .map(PathBuf::from)
            else {
                continue;
            };
            let catalog = [
                ".agents/plugins/marketplace.json",
                "marketplace.json",
                ".claude-plugin/marketplace.json",
            ]
            .iter()
            .map(|p| root.join(p))
            .find(|p| p.is_file());
            if let Some(catalog) = catalog {
                selected.insert(alias.clone(), (root, catalog));
            } else {
                warnings.push(format!("Marketplace {alias} catalog is unavailable."));
            }
        }
    }
    selected
        .into_iter()
        .flat_map(|(alias, (root, catalog))| {
            catalog_registrations(&alias, &root, &catalog, warnings)
        })
        .collect()
}
pub fn scan(ctx: &ScanContext) -> Result<CodexScanReport> {
    ctx.validate()?;
    let mut report = codex_scanner::scan_with_paths(
        Some(ctx.user_home.clone()),
        Some(ctx.codex_home.clone()),
        ctx.project_root.clone(),
    );
    if report.warnings.iter().any(|warning| {
        let warning = warning.to_lowercase();
        warning.contains("could not")
            || warning.contains("invalid toml")
            || warning.contains("entry limit")
    }) {
        return Err(
            "Required scan metadata is unreadable; the previous installation state was preserved."
                .into(),
        );
    }
    enrich(&mut report, ctx)?;
    Ok(report)
}
pub fn enrich(report: &mut CodexScanReport, ctx: &ScanContext) -> Result<()> {
    ctx.validate()?;
    let mut warnings = Vec::new();
    let registrations = registrations(ctx, &mut warnings);
    if !warnings.is_empty() {
        return Err(format!(
            "Source resolution is incomplete: {}",
            warnings.join(" ")
        ));
    }
    let mut obs = Vec::new();
    let mut plugins = BTreeMap::<PathBuf, Observation>::new();
    // Resolve parents first so bundled Skill identity inherits an evidenced plugin component.
    for entity in report.entities.iter().filter(|e| e.kind == "plugin") {
        let path = PathBuf::from(
            entity.evidence["installationPath"]
                .as_str()
                .ok_or("Plugin location evidence is missing.")?,
        );
        let manifest = entity.evidence["manifestPath"].as_str().map(PathBuf::from);
        let parsed = manifest.as_ref().and_then(|p| json_file(p).ok());
        let component = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        let market = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let mut o = provisional(entity, &ctx.user_key, path.clone(), manifest.clone());
        o.cache_version = path.file_name().map(|s| s.to_string_lossy().into_owned());
        if let Some((value, sha)) = parsed {
            o.manifest_hash = sha;
            o.package_version = value["version"].as_str().map(str::to_owned);
            o.machine_name = value["name"].as_str().unwrap_or(&entity.name).into();
            let matching: Vec<_> = registrations
                .iter()
                .filter(|r| {
                    r.alias == market
                        && r.machine == component
                        && r.machine == o.machine_name
                        && r.source_core == core_manifest(&value)
                })
                .collect();
            if matching.len() == 1 {
                let r = matching[0];
                o.authority = r.origin.clone();
                o.component_key = r.component.clone();
                o.confidence = "SOURCE_COMPONENT_VERIFIED".into();
                o.evidence = json!({"marketplace":r.alias,"catalog":r.catalog,"component":r.component,"sourceManifestHash":r.source_hash,"cacheManifestHash":o.manifest_hash,"sourceManifestMatchesCache":r.source_hash==o.manifest_hash,"sourceVersion":r.source_version,"identityFieldsMatch":true,"sourceRoot":r.source_root,"catalogPath":r.catalog_path,"runtimeVerified":false});
                resolve(&mut o);
            } else {
                o.evidence = json!({"reason":"No unique configured catalog/source-manifest association; no name-only merge.","runtimeVerified":false});
            }
        }
        physical(&mut o);
        plugins.insert(path, o.clone());
        obs.push(o);
    }
    for entity in report.entities.iter().filter(|e| e.kind != "plugin") {
        if entity.kind == "skill" {
            let manifest = PathBuf::from(
                entity.evidence["manifestPath"]
                    .as_str()
                    .ok_or("Skill location evidence is missing.")?,
            );
            let location = manifest
                .parent()
                .ok_or("Invalid Skill location.")?
                .to_path_buf();
            let scope = entity.evidence["scope"].as_str().unwrap_or("");
            let scope_key = if scope == "project" {
                ctx.project_key
                    .as_ref()
                    .ok_or("Project scope was not selected.")?
            } else {
                &ctx.user_key
            };
            let mut o = provisional(entity, scope_key, location.clone(), Some(manifest.clone()));
            o.manifest_hash = digest(&read(&manifest, 65536)?);
            if scope == "plugin" {
                if let Some((plugin_path, parent)) =
                    plugins.iter().find(|(p, _)| manifest.starts_with(p))
                {
                    if parent.confidence == "SOURCE_COMPONENT_VERIFIED" {
                        let relative = manifest
                            .strip_prefix(plugin_path)
                            .map_err(|_| "Bundled component is outside its plugin.")?
                            .to_string_lossy()
                            .replace('\\', "/");
                        o.authority = parent.authority.clone();
                        o.component_key = format!("{}#{}", parent.component_key, relative);
                        o.package_version = parent.package_version.clone();
                        o.cache_version = parent.cache_version.clone();
                        o.confidence = parent.confidence.clone();
                        o.evidence = json!({"parentLogicalId":parent.logical_id,"parentPhysicalId":parent.physical_id,"pluginLocation":plugin_path,"component":relative,"sourceComponentVerified":true,"runtimeVerified":false});
                        resolve(&mut o);
                    }
                }
            } else {
                let root = match scope {
                    "project" => ctx.project_root.as_ref().map(|p| p.join(".agents/skills")),
                    "user" => Some(ctx.user_home.join(".agents/skills")),
                    _ => Some(ctx.codex_home.join("skills")),
                }
                .ok_or("Skill root unavailable.")?;
                let canonical_root = root
                    .canonicalize()
                    .map_err(|_| "Selected Skill root is unavailable.")?;
                let canonical_location = location
                    .canonicalize()
                    .map_err(|_| "Observed Skill location is unavailable.")?;
                let relative = canonical_location
                    .strip_prefix(&canonical_root)
                    .map_err(|_| "Skill is outside its selected scope.")?
                    .to_string_lossy()
                    .replace('\\', "/");
                o.authority =
                    json!({"type":"scoped-standalone-skill","scope":scope,"scopeKey":scope_key});
                o.component_key = relative;
                o.confidence = "SCOPED_COMPONENT".into();
                o.evidence = json!({"selectedRoot":root,"scopeKey":scope_key,"manifestHash":o.manifest_hash,"runtimeVerified":false});
                resolve(&mut o);
            }
            physical(&mut o);
            obs.push(o);
        } else if entity.kind == "mcp" {
            let config_path = PathBuf::from(
                entity.evidence["configPath"]
                    .as_str()
                    .ok_or("MCP configuration evidence missing.")?,
            );
            let scope = entity.evidence["scope"].as_str().unwrap_or("user");
            let scope_key = if scope == "project" {
                ctx.project_key
                    .as_ref()
                    .ok_or("Project scope unavailable.")?
            } else {
                &ctx.user_key
            };
            let bytes = read(&config_path, 1024 * 1024)?;
            let config: toml::Value = toml::from_str(
                std::str::from_utf8(&bytes).map_err(|_| "Invalid configuration encoding.")?,
            )
            .map_err(|_| "Invalid configuration TOML.")?;
            let entry = config
                .get("mcp_servers")
                .and_then(|s| s.get(&entity.name))
                .ok_or("MCP entry disappeared during scan.")?;
            let mut o = provisional(entity, scope_key, config_path.clone(), None);
            o.authority = json!({"type":"scoped-mcp-entry","scope":scope,"scopeKey":scope_key});
            o.component_key = entity.name.clone();
            o.confidence = "SCOPED_COMPONENT".into();
            o.manifest_hash = hash(&entry.to_string());
            o.package_version = Some(format!("configuration:{}", o.manifest_hash));
            o.evidence = json!({"configPath":config_path,"entryName":entity.name,"declarationHash":o.manifest_hash,"connected":false,"runtimeVerified":false});
            resolve(&mut o);
            physical(&mut o);
            obs.push(o);
        }
    }
    obs.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    for entity in &mut report.entities {
        if let Some(o) = obs.iter().find(|o| o.entity_id == entity.id) {
            entity.evidence["logicalId"] = json!(o.logical_id);
            entity.evidence["physicalId"] = json!(o.physical_id);
            entity.evidence["scopeKey"] = json!(o.scope_key);
        }
    }
    let mut duplicate_manifests = Vec::new();
    let mut manifest_groups = BTreeMap::<String, Vec<&Observation>>::new();
    for o in obs
        .iter()
        .filter(|o| o.kind == "skill" && !o.manifest_hash.is_empty())
    {
        manifest_groups
            .entry(o.manifest_hash.clone())
            .or_default()
            .push(o);
    }
    for (manifest_hash, group) in manifest_groups {
        if group
            .iter()
            .map(|o| &o.logical_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1
        {
            duplicate_manifests.push(json!({"manifestHash":manifest_hash,"logicalIds":group.iter().map(|o|&o.logical_id).collect::<Vec<_>>(),"relation":"IDENTICAL_MANIFEST_ONLY_DISTINCT_PROVENANCE","merged":false}));
        }
    }
    report.identity = Some(IdentityScan {
        context: ctx.clone(),
        observations: obs,
        history: vec![],
        warnings,
        duplicate_manifests,
        operation_id: None,
    });
    Ok(())
}
fn provisional(
    e: &codex_scanner::ScannedEntity,
    scope: &str,
    location: PathBuf,
    manifest_path: Option<PathBuf>,
) -> Observation {
    let origin = json!({"type":"unproven-physical-origin","legacyEntityId":e.id});
    let origin_key = hash(&origin.to_string());
    Observation {
        entity_id: e.id.clone(),
        physical_id: String::new(),
        logical_id: format!("logical:{}:{}", e.kind, hash(&e.id)),
        kind: e.kind.clone(),
        machine_name: e.name.clone(),
        origin_key,
        authority: origin,
        component_key: e.id.clone(),
        scope_key: scope.into(),
        location,
        manifest_path,
        package_version: None,
        cache_version: None,
        manifest_hash: String::new(),
        present: true,
        status: if e.kind == "mcp" {
            "UNKNOWN".into()
        } else {
            e.status.as_str().into()
        },
        confidence: "PROVISIONAL_NO_MERGE".into(),
        evidence: json!({"runtimeVerified":false}),
    }
}
fn resolve(o: &mut Observation) {
    o.origin_key = hash(&o.authority.to_string());
    o.logical_id = format!(
        "logical:{}:{}",
        o.kind,
        hash(
            &json!([
                o.kind,
                o.origin_key,
                o.component_key,
                o.machine_name.to_lowercase()
            ])
            .to_string()
        )
    );
}
fn physical(o: &mut Observation) {
    o.physical_id = format!(
        "physical:{}",
        hash(
            &json!([
                o.logical_id,
                normalized(&o.location),
                o.package_version,
                o.manifest_hash
            ])
            .to_string()
        )
    );
}
pub(crate) fn historical(
    report: &CodexScanReport,
    old: &CodexScanReport,
) -> BTreeMap<String, Observation> {
    let mut rows = BTreeMap::new();
    let Some(scan) = report.identity.as_ref() else {
        return rows;
    };
    for current in scan
        .observations
        .iter()
        .filter(|o| o.kind == "plugin" && o.confidence == "SOURCE_COMPONENT_VERIFIED")
    {
        for previous in old
            .plugins
            .iter()
            .filter(|v| v.valid && v.name == current.machine_name)
        {
            let Some(version) = previous.version.as_ref() else {
                continue;
            };
            let Some(parent) = current.location.parent() else {
                continue;
            };
            let old_path = parent.join(version);
            if codex_scanner::stable_id("plugin", &old_path) != previous.id {
                continue;
            }
            let mut o = current.clone();
            o.entity_id = previous.id.clone();
            o.location = old_path.clone();
            o.package_version = previous.version.clone();
            o.cache_version = previous.version.clone();
            o.manifest_path = Some(
                if current
                    .manifest_path
                    .as_ref()
                    .is_some_and(|p| p.ends_with(".codex-plugin/plugin.json"))
                {
                    old_path.join(".codex-plugin/plugin.json")
                } else {
                    old_path.join("plugin.json")
                },
            );
            o.manifest_hash = String::new();
            o.present = false;
            o.status = "NOT_INSTALLED".into();
            o.evidence = json!({"proof":"EXACT_HISTORICAL_PATH_ID_AND_SAVED_VALID_MANIFEST_METADATA","cacheNamespaceMatches":true,"registeredSourceComponent":current.component_key,"currentManifestProvenance":current.evidence,"historicalManifestBytesAvailable":false,"runtimeVerified":false});
            physical(&mut o);
            rows.insert(o.entity_id.clone(), o);
            for bundled in scan.observations.iter().filter(|s| {
                s.kind == "skill" && s.evidence["parentLogicalId"] == current.logical_id
            }) {
                let Some(manifest) = bundled.manifest_path.as_ref() else {
                    continue;
                };
                let Ok(relative) = manifest.strip_prefix(&current.location) else {
                    continue;
                };
                let old_manifest = old_path.join(relative);
                #[cfg(windows)]
                let key = PathBuf::from(format!("\\\\?\\{}", old_manifest.display()));
                #[cfg(not(windows))]
                let key = old_manifest.clone();
                let id = codex_scanner::stable_id("skill", key);
                if !old
                    .skills
                    .iter()
                    .any(|s| s.id == id && s.name == bundled.machine_name && s.valid)
                {
                    continue;
                }
                let mut o = bundled.clone();
                o.entity_id = id;
                o.location = old_manifest.parent().unwrap().into();
                o.manifest_path = Some(old_manifest);
                o.package_version = previous.version.clone();
                o.cache_version = previous.version.clone();
                o.manifest_hash = String::new();
                o.present = false;
                o.status = "NOT_INSTALLED".into();
                o.evidence = json!({"proof":"EXACT_HISTORICAL_BUNDLED_PATH_ID_AND_SAVED_VALID_MANIFEST_METADATA","parentLogicalId":current.logical_id,"historicalManifestBytesAvailable":false,"runtimeVerified":false});
                physical(&mut o);
                rows.insert(o.entity_id.clone(), o);
            }
        }
    }
    rows
}
