//! Reviewed third-party compatibility profiles. Data never grants execution rights.
use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(super) mod remote;
mod skill;
#[cfg(test)]
mod tests;
#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub root: String,
    pub component: String,
    pub repository: String,
    pub commit: String,
    pub enabled: bool,
    pub mode: String,
    pub files: BTreeMap<String, String>,
    pub tool: Value,
    pub endpoint: String,
    pub plugin: Value,
    pub reviewed_at: i64,
    pub blockers: Vec<String>,
}
impl Profile {
    pub fn digest(&self) -> String {
        digest(serde_json::to_string(self).unwrap().as_bytes())
    }
}
fn folder() -> PathBuf {
    security::root().join("compatibility")
}
pub fn project_inputs(root: &str) -> Result<Value> {
    let root = security::project(root)?;
    let mut out = serde_json::Map::new();
    for name in ["sample.tsx", "sample.js", "input.json"] {
        let path = root.join(name);
        if path.is_file() {
            out.insert(name.into(), json!(digest(&security::read(&path, 32768)?)));
        }
    }
    Ok(Value::Object(out))
}
pub fn sanitized(value: &Value) -> Value {
    match value {
        Value::String(s) => json!(clean(s)),
        Value::Array(xs) => Value::Array(xs.iter().map(sanitized).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), sanitized(v))).collect())
        }
        _ => value.clone(),
    }
}
pub fn public_inputs(plan: &Plan) -> Result<Value> {
    let mut inputs = serde_json::Map::new();
    for n in &plan.nodes {
        if n.adapter == "mcp.reviewed" {
            let p = validate(n)?;
            let tool = p.tool["name"].as_str().ok_or("MCP_TOOL_NAME")?;
            inputs.insert(p.id, remote::public_arguments(tool, &plan.goal)?);
        }
    }
    Ok(Value::Object(inputs))
}
fn ident(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('\\')
        && !s.contains(':')
        && s.split('/').all(|x| !x.is_empty() && x != ".." && x != ".")
}
fn name(path: &Path) -> Result<String> {
    let raw = String::from_utf8(security::read(path, 65536)?).map_err(|_| "SKILL_UTF8")?;
    let front = raw
        .strip_prefix("---")
        .and_then(|s| s.split_once("---").map(|x| x.0))
        .ok_or("SKILL_FRONTMATTER")?;
    let docs = yaml_rust2::YamlLoader::load_from_str(front).map_err(|_| "SKILL_FRONTMATTER")?;
    let name = docs
        .first()
        .and_then(|d| d["name"].as_str())
        .ok_or("SKILL_NAME")?;
    if !ident(name) {
        return Err("SKILL_NAME".into());
    }
    Ok(name.into())
}
fn inventory(
    root: &Path,
    here: &Path,
    out: &mut BTreeMap<String, String>,
    total: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        return Err("PACKAGE_DEPTH_LIMIT".into());
    }
    let mut entries = std::fs::read_dir(here)
        .map_err(|_| "PACKAGE_READ")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "PACKAGE_READ")?;
    entries.sort_by_key(|x| x.path());
    for entry in entries {
        let p = entry.path();
        if !security::ordinary(&p) {
            return Err("PACKAGE_LINK_REJECTED".into());
        }
        if p.is_dir() {
            if entry.file_name() == ".git" || entry.file_name() == "node_modules" {
                continue;
            }
            inventory(root, &p, out, total, depth + 1)?;
        } else {
            let bytes = security::read(&p, 1_500_000)?;
            static CREDENTIALS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
                || {
                    regex::Regex::new(r"(?i)(?:sk-(?:proj-)?[a-z0-9_-]{20,}|ghp_[a-z0-9]{30,}|github_pat_[a-z0-9_]{30,}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)").unwrap()
                },
            );
            if CREDENTIALS.is_match(&String::from_utf8_lossy(&bytes)) {
                return Err("SENSITIVE_SOURCE_REVIEW_REQUIRED_NO_COPY_OR_MODEL_CALL".into());
            }
            *total += bytes.len();
            if *total > 1500000 || out.len() >= 200 {
                return Err("PACKAGE_SIZE_LIMIT".into());
            }
            let rel = p
                .strip_prefix(root)
                .map_err(|_| "PACKAGE_PATH")?
                .to_string_lossy()
                .replace('\\', "/");
            if !relative(&rel) {
                return Err("PACKAGE_PATH".into());
            }
            out.insert(rel, digest(&bytes));
        }
    }
    Ok(())
}
fn executable_dependency(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let basename = path.rsplit('/').next().unwrap_or_default();
    path.split('/')
        .any(|part| matches!(part, "scripts" | "bin" | "hooks"))
        || matches!(
            basename,
            "package.json" | "requirements.txt" | "pyproject.toml" | "cargo.toml"
        )
        || basename.rsplit_once('.').is_some_and(|(_, ext)| {
            matches!(
                ext,
                "exe"
                    | "dll"
                    | "msi"
                    | "com"
                    | "bat"
                    | "cmd"
                    | "ps1"
                    | "sh"
                    | "bash"
                    | "py"
                    | "rb"
                    | "jar"
                    | "wasm"
                    | "vbs"
            )
        })
}
pub fn check(p: &Profile) -> Result<()> {
    if p.kind == "plugin" {
        if let Some(home) = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .and_then(|p| std::fs::canonicalize(p).ok())
        {
            let cache = home.join("plugins/cache");
            if let Ok(relative) = Path::new(&p.root).strip_prefix(&cache) {
                let market = relative
                    .components()
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .ok_or("PLUGIN_ENABLE_STATE_NOT_VERIFIED")?;
                let raw = security::read(&home.join("config.toml"), 262144)?;
                let cfg: toml::Value =
                    toml::from_str(std::str::from_utf8(&raw).map_err(|_| "PLUGIN_CONFIG_UTF8")?)
                        .map_err(|_| "PLUGIN_CONFIG_PARSE")?;
                let key = format!(
                    "{}@{market}",
                    p.plugin["name"].as_str().ok_or("PLUGIN_NAME")?
                );
                match cfg
                    .get("plugins")
                    .and_then(|v| v.get(&key))
                    .and_then(|v| v.get("enabled"))
                    .and_then(toml::Value::as_bool)
                {
                    Some(true) => {}
                    Some(false) => return Err("PLUGIN_DISABLED_NO_EXECUTION".into()),
                    None => return Err("PLUGIN_ENABLE_STATE_NOT_VERIFIED".into()),
                }
            }
        }
    }
    if !p.enabled {
        return Err("PLUGIN_DISABLED_NO_EXECUTION".into());
    }
    if p.kind == "plugin" && p.plugin["structure"] != "SKILL_ONLY" {
        return Err("PLUGIN_PROVIDER_RUNTIME_NOT_VERIFIED".into());
    }
    if !p.blockers.is_empty() {
        return Err(p.blockers.join(";"));
    }
    if p.kind != "mcp" {
        let root = security::project(&p.root)?;
        let mut current = BTreeMap::new();
        inventory(&root, &root, &mut current, &mut 0, 0)?;
        if current != p.files {
            return Err("CAPABILITY_VERSION_CHANGED_REVIEW_REQUIRED".into());
        }
        if current.keys().any(|path| executable_dependency(path)) {
            return Err("DEPENDENCY_AND_SCRIPT_EXECUTION_NOT_APPROVED".into());
        }
    }
    Ok(())
}
pub fn load(id: &str, expected: &str) -> Result<Profile> {
    if !ident(id) || expected.len() != 64 {
        return Err("REVIEW_ID_OR_DIGEST".into());
    }
    let bytes = security::read(&folder().join(format!("{id}.json")), 200000)?;
    let p: Profile = serde_json::from_slice(&bytes).map_err(|_| "INVALID_REVIEW")?;
    if p.id != id || p.digest() != expected {
        return Err("REVIEW_DIGEST_MISMATCH".into());
    }
    Ok(p)
}
pub fn validate(n: &Node) -> Result<Profile> {
    if n.risk != "MEDIUM"
        || n.max_attempts != 1
        || n.retry_policy != "NEVER"
        || n.capability_id.is_some()
        || n.params.as_object().is_none_or(|p| p.len() != 2)
    {
        return Err("REVIEWED_SCOPE_MISMATCH".into());
    }
    let p = load(
        n.params["profileId"].as_str().ok_or("REVIEW_ID")?,
        n.params["reviewDigest"].as_str().ok_or("REVIEW_DIGEST")?,
    )?;
    if n.capability_type != p.kind
        || n.capability_version.as_deref() != Some(p.digest().as_str())
        || n.adapter
            != match p.kind.as_str() {
                "skill" => "skill.reviewed",
                "plugin" => "plugin.reviewed",
                "mcp" => "mcp.reviewed",
                _ => return Err("REVIEW_KIND".into()),
            }
    {
        return Err("REVIEWED_SCOPE_MISMATCH".into());
    }
    Ok(p)
}
fn summary(p: &Profile) -> Value {
    let failure = check(p).err();
    let files_present = p.kind != "mcp"
        && !p.files.is_empty()
        && security::project(&p.root).is_ok_and(|root| {
            p.files.keys().all(|rel| {
                relative(rel) && {
                    let path = root.join(rel);
                    path.is_file() && security::ordinary(&path)
                }
            })
        });
    let blockers = if let Some(error) = failure.clone() {
        vec![error]
    } else {
        vec![]
    };
    json!({"id":p.id,"kind":p.kind,"name":p.name,"digest":p.digest(),"repository":p.repository,"commit":p.commit,"mode":p.mode,"enabled":p.enabled,"blockers":blockers,"bundle":p.plugin,"reviewedAt":p.reviewed_at,"policyReady":failure.is_none(),"transport":if p.kind=="mcp"{"streamable-http"}else{"codex-app-server"},"states":{"DISCOVERED":true,"FILES_PRESENT":files_present,"INSTALLED":"NOT VERIFIED","CONFIGURED":p.kind=="mcp","CODEX_RECOGNIZED":"NOT VERIFIED","LOADED":"NOT VERIFIED","CONNECTED":"NOT VERIFIED","CALLABLE":"NOT VERIFIED","EXECUTED":"NOT VERIFIED","VERIFIED":"NOT VERIFIED"},"permission":"Instruction-only model analysis or reviewed public read-only MCP; no shell, private accounts or global config writes"})
}
pub fn list() -> Result<Value> {
    let mut newest: BTreeMap<String, Profile> = BTreeMap::new();
    let mut historical = 0;
    if folder().is_dir() {
        for e in std::fs::read_dir(folder()).map_err(|_| "REVIEW_READ")? {
            let e = e.map_err(|_| "REVIEW_READ")?;
            if e.path().extension().is_some_and(|x| x == "json") {
                if let Ok(b) = security::read(&e.path(), 200000) {
                    if let Ok(p) = serde_json::from_slice::<Profile>(&b) {
                        historical += 1;
                        let key = format!(
                            "{}|{}|{}|{}|{}|{}",
                            p.root, p.component, p.endpoint, p.mode, p.enabled, p.tool["name"]
                        );
                        if newest
                            .get(&key)
                            .is_none_or(|old| old.reviewed_at <= p.reviewed_at)
                        {
                            newest.insert(key, p);
                        }
                    }
                }
            }
        }
    }
    Ok(
        json!({"profiles":newest.values().map(summary).collect::<Vec<_>>(),"historicalReviewsRetained":historical}),
    )
}
pub fn suggestions(goal: &str) -> Vec<Value> {
    let q = goal.to_lowercase();
    if ["disabled", "禁用", "未配置", "unconfigured"]
        .iter()
        .any(|x| q.contains(x))
    {
        return vec![];
    }
    let mut choices = Vec::new();
    let Ok(list) = list() else { return choices };
    let mut scored = Vec::new();
    for summary in list["profiles"].as_array().into_iter().flatten() {
        if summary["policyReady"] != true {
            continue;
        }
        let name = summary["name"].as_str().unwrap_or("").to_lowercase();
        let kind = summary["kind"].as_str().unwrap_or("");
        let score = if kind == "mcp"
            && ["mcp", "查询", "documentation", "文档查询"]
                .iter()
                .any(|s| q.contains(s))
        {
            30
        } else if kind == "plugin"
            && ["plugin", "插件", "expo", "swift"]
                .iter()
                .any(|s| q.contains(s))
        {
            30
        } else if kind == "skill"
            && name.contains("composition")
            && ["composition", "boolean", "组件", "布尔"]
                .iter()
                .any(|s| q.contains(s))
        {
            28
        } else if kind == "skill"
            && name.contains("frontend")
            && ["ui", "react", "界面", "交互"]
                .iter()
                .any(|s| q.contains(s))
        {
            25
        } else {
            0
        };
        if score > 0 {
            scored.push((score, summary.clone()))
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1["reviewedAt"].as_i64().cmp(&a.1["reviewedAt"].as_i64()))
    });
    let multiple = ["组合", "多个", "combined", "then", "and"]
        .iter()
        .any(|s| q.contains(s));
    let mut kinds = std::collections::BTreeSet::new();
    for (_, p) in scored {
        let kind = p["kind"].as_str().unwrap_or("");
        if kinds.insert(kind.to_string()) {
            choices.push(json!({"id":p["id"],"digest":p["digest"]}));
            if choices.len() == if multiple { 2 } else { 1 } {
                break;
            }
        }
    }
    choices.sort_by_key(|p| {
        load(
            p["id"].as_str().unwrap_or(""),
            p["digest"].as_str().unwrap_or(""),
        )
        .map(|p| if p.kind == "mcp" { 0 } else { 1 })
        .unwrap_or(2)
    });
    choices
}
pub async fn review(args: &Value) -> Result<Value> {
    let kind = text(args, "kind")?;
    if !matches!(kind, "skill" | "plugin" | "mcp") {
        return Err("REVIEW_KIND".into());
    }
    let repository = text(args, "repository")?.to_string();
    let commit = text(args, "commit")?.to_string();
    if kind == "mcp" {
        if args["networkApproved"] != true {
            return Err("MCP_DISCOVERY_NETWORK_APPROVAL_REQUIRED".into());
        }
        if repository != "https://developers.openai.com/learn/docs-mcp"
            || commit != "NOT_PUBLISHED_HOSTED_SERVICE"
        {
            return Err("HOSTED_MCP_PROVENANCE_MUST_NOT_INVENT_GIT_COMMIT".into());
        }
    } else if !repository.starts_with("https://github.com/")
        || repository.len() > 300
        || commit.len() != 40
        || !commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("FIXED_GITHUB_PROVENANCE_REQUIRED".into());
    }
    if kind != "mcp" {
        let url = reqwest::Url::parse(&repository).map_err(|_| "SOURCE_REPOSITORY_URL")?;
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        if url.host_str() != Some("github.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || parts.len() != 2
            || parts.iter().any(|s| {
                s.is_empty()
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            })
        {
            return Err("SOURCE_REPOSITORY_URL_NO_CREDENTIALS_OR_QUERY".into());
        }
    }
    let mut p = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.into(),
        name: String::new(),
        root: String::new(),
        component: String::new(),
        repository,
        commit,
        enabled: args["enabled"] != false,
        mode: "INSTRUCTION_ONLY".into(),
        files: BTreeMap::new(),
        tool: Value::Null,
        endpoint: String::new(),
        plugin: Value::Null,
        reviewed_at: now(),
        blockers: vec![],
    };
    if kind == "mcp" {
        p.name = text(args, "name")?.into();
        p.endpoint = text(args, "endpoint")?.into();
        if p.endpoint != "https://developers.openai.com/mcp" {
            return Err("HOSTED_MCP_ENDPOINT_DOES_NOT_MATCH_PROVENANCE".into());
        }
        p.mode = "PUBLIC_READ_ONLY_MCP".into();
        remote::endpoint(&p.endpoint)?;
        let tool = text(args, "tool")?;
        let discovery = remote::discover(&p.endpoint, Arc::new(AtomicU8::new(0))).await?;
        p.tool = discovery["tools"]
            .as_array()
            .and_then(|xs| xs.iter().find(|t| t["name"] == tool))
            .cloned()
            .ok_or("MCP_TOOL_NOT_DISCOVERED")?;
        remote::tool_policy(&p.tool)?;
    } else {
        let root = security::project(text(args, "root")?)?;
        p.root = root.to_string_lossy().into_owned();
        p.component = if kind == "skill" {
            "SKILL.md".into()
        } else {
            args["component"].as_str().unwrap_or_default().trim().into()
        };
        if kind == "plugin" {
            let manifest = [
                "plugin.json",
                ".codex-plugin/plugin.json",
                ".claude-plugin/plugin.json",
            ]
            .into_iter()
            .find(|n| root.join(n).is_file())
            .ok_or("PLUGIN_MANIFEST_MISSING")?;
            let m: Value = serde_json::from_slice(&security::read(&root.join(manifest), 65536)?)
                .map_err(|_| "PLUGIN_MANIFEST_INVALID")?;
            let plugin_name = m["name"]
                .as_str()
                .filter(|n| ident(n))
                .ok_or("PLUGIN_MANIFEST_INVALID")?;
            if m.get("skills")
                .is_some_and(|s| !matches!(s.as_str(), Some("./skills/" | "./skills")))
            {
                return Err("PLUGIN_SKILL_PATH_UNSUPPORTED".into());
            }
            let extension = m.pointer("/extensions/com.openai").unwrap_or(&m);
            let has_mcp = root.join("mcp.json").is_file()
                || root.join(".mcp.json").is_file()
                || m.get("mcpServers").is_some()
                || extension.get("mcpServers").is_some();
            let has_apps = root.join(".app.json").is_file()
                || m.get("apps").is_some()
                || extension.get("apps").is_some();
            let has_hooks = m.get("hooks").is_some()
                || extension.get("hooks").is_some()
                || root.join("hooks/hooks.json").is_file();
            if has_hooks {
                p.blockers
                    .push("PLUGIN_HOOKS_REQUIRE_SEPARATE_REVIEW".into())
            }
            p.name = plugin_name.into();
            p.plugin = json!({"name":m["name"],"version":m["version"],"manifest":manifest,"providerResources":has_mcp||has_apps,
                "hasMcp":has_mcp,"hasApps":has_apps,"hasHooks":has_hooks,"mcpExecuted":false,
                "accountRequirement":if has_mcp||has_apps {"PROVIDER_AUTH_NOT_VERIFIED"} else {"NO_PROVIDER_COMPONENT_DECLARED"}});
        }
        inventory(&root, &root, &mut p.files, &mut 0, 0)?;
        if p.files.keys().any(|path| executable_dependency(path)) {
            p.blockers
                .push("DEPENDENCY_AND_SCRIPT_EXECUTION_NOT_APPROVED".into());
        }
        if kind == "plugin" {
            let skills = p
                .files
                .keys()
                .filter(|rel| rel.starts_with("skills/") && rel.ends_with("/SKILL.md"))
                .cloned()
                .collect::<Vec<_>>();
            let has_resources = p.files.keys().any(|rel| {
                ["assets/", "templates/", "resources/"]
                    .iter()
                    .any(|prefix| rel.starts_with(prefix))
            });
            let structure = match (!skills.is_empty(), p.plugin["providerResources"] == true) {
                (true, true) => "MIXED",
                (true, false) => "SKILL_ONLY",
                (false, true) => "MCP_BACKED",
                (false, false) if has_resources => "RESOURCE_ONLY",
                (false, false) => return Err("PLUGIN_NO_SUPPORTED_COMPONENTS".into()),
            };
            p.plugin["structure"] = json!(structure);
            p.plugin["bundledSkills"] = json!(skills);
            p.plugin["hasResources"] = json!(has_resources);
            if p.component.is_empty() && structure == "SKILL_ONLY" {
                return Err("PLUGIN_COMPONENT_PATH".into());
            }
        }
        if !p.component.is_empty() {
            if !relative(&p.component)
                || Path::new(&p.component).file_name().and_then(|n| n.to_str()) != Some("SKILL.md")
                || (kind == "plugin" && !p.component.starts_with("skills/"))
            {
                return Err("PLUGIN_COMPONENT_PATH".into());
            }
            if !p.files.contains_key(&p.component) {
                return Err("SKILL_FILE_MISSING".into());
            }
            p.name = name(&root.join(&p.component))?;
        }
        if args["mode"] == "FULL_TOOL_EXECUTION" {
            p.mode = "FULL_TOOL_EXECUTION".into();
            if !p
                .blockers
                .iter()
                .any(|item| item == "DEPENDENCY_AND_SCRIPT_EXECUTION_NOT_APPROVED")
            {
                p.blockers
                    .push("DEPENDENCY_AND_SCRIPT_EXECUTION_NOT_APPROVED".into());
            }
        }
    }
    security::owned_dir(&folder())?;
    security::seal(
        &folder().join(format!("{}.json", p.id)),
        &serde_json::to_vec_pretty(&p).map_err(|_| "REVIEW_SERIALIZE")?,
    )?;
    Ok(summary(&p))
}
pub fn node(p: &Profile, id: &str, deps: &[&str]) -> Node {
    let mut n = types::base_node(
        id,
        match p.kind.as_str() {
            "skill" => "skill.reviewed",
            "plugin" => "plugin.reviewed",
            _ => "mcp.reviewed",
        },
        &format!("{} — {}", p.name, p.mode),
        deps,
    );
    n.capability_type = p.kind.clone();
    n.capability_version = Some(p.digest());
    n.params = json!({"profileId":p.id,"reviewDigest":p.digest()});
    n.risk = "MEDIUM".into();
    n.retry_policy = "NEVER".into();
    n.timeout_ms = 120000;
    n.estimated_cost = if p.kind == "mcp" {
        "Public read-only MCP network call; no account or model"
    } else {
        "One existing ChatGPT model turn; instruction-only; no tools"
    }
    .into();
    n
}
pub async fn execute_skill(c: adapters::Context<'_>) -> Result<Value> {
    let p = validate(c.node)?;
    check(&p)?;
    if p.kind != "skill" {
        return Err("SKILL_ADAPTER_TYPE".into());
    }
    let goal = c.plan.goal.clone();
    let project = c.plan.project_root.clone();
    let work = c.workspace.to_path_buf();
    let inputs = c.inputs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        skill::execute(&p, &goal, &project, &work, &inputs, c.control, c.progress)
    })
    .await
    .map_err(|_| "REVIEWED_SKILL_TASK")?
}
pub async fn execute_plugin(c: adapters::Context<'_>) -> Result<Value> {
    let p = validate(c.node)?;
    check(&p)?;
    if p.kind != "plugin" {
        return Err("PLUGIN_ADAPTER_TYPE".into());
    }
    if p.plugin["structure"] != "SKILL_ONLY" {
        return Err("PLUGIN_MCP_OR_ACCOUNT_COMPONENT_NOT_APPROVED_NOT_VERIFIED".into());
    }
    let goal = c.plan.goal.clone();
    let project = c.plan.project_root.clone();
    let work = c.workspace.to_path_buf();
    let inputs = c.inputs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = c.progress.send("NODE_CONNECTING");
        let recognition = probe_plugin_in(&p, work.join("plugin-registration"))?;
        let mut result =
            skill::execute(&p, &goal, &project, &work, &inputs, c.control, c.progress)?;
        result["pluginRecognition"] = recognition;
        result["states"]["CODEX_PLUGIN_RECOGNIZED"] = json!(true);
        Ok(result)
    })
    .await
    .map_err(|_| "REVIEWED_PLUGIN_TASK")?
}
pub async fn execute_mcp(c: adapters::Context<'_>) -> Result<Value> {
    let p = validate(c.node)?;
    check(&p)?;
    if p.kind != "mcp" {
        return Err("MCP_ADAPTER_TYPE".into());
    }
    remote::execute(&p, &c.plan.goal, c.control, c.progress).await
}
pub fn scope(n: &Node) -> Value {
    match validate(n) {
        Ok(p) => {
            let mcp = p.kind == "mcp";
            json!({"profileId":p.id,"reviewDigest":p.digest(),"kind":p.kind,"name":p.name,"repository":p.repository,"commit":p.commit,
                "sourceRoot":p.root,"readFiles":p.files.keys().collect::<Vec<_>>(),"endpoint":p.endpoint,"tool":p.tool["name"],
                "inputSchema":p.tool["inputSchema"],"outputSchema":p.tool["outputSchema"],"mode":p.mode,"plugin":p.plugin,
                "risk":n.risk,"estimatedCost":n.estimated_cost,"modelCalls":!mcp,"noShell":true,"noGlobalConfigWrites":true,
                "process":if mcp{"No child process"}else{"Pinned Codex App Server"},
                "network":if mcp{"Reviewed public endpoint only"}else{"Codex inference only; sandbox tool network disabled"},
                "account":if mcp{"None"}else{"Existing Codex account quota"},
                "writes":"Application-owned execution directory and audit rows",
                "expectedOutput":if mcp{"Schema-checked documentation response"}else{"Structured instruction-only analysis"}})
        }
        Err(_) => json!({"error":"REVIEW_UNAVAILABLE"}),
    }
}

pub fn probe_plugin(id: &str, expected: &str) -> Result<Value> {
    let p = load(id, expected)?;
    probe_plugin_in(
        &p,
        folder()
            .join("plugin-probes")
            .join(uuid::Uuid::new_v4().to_string()),
    )
}
fn probe_plugin_in(p: &Profile, work: PathBuf) -> Result<Value> {
    check(p)?;
    if p.kind != "plugin" || p.plugin["structure"] != "SKILL_ONLY" {
        return Err("ONLY_REVIEWED_SKILL_ONLY_PLUGIN_PROBE_NO_MCP_OR_HOOKS".into());
    }
    let name = p.plugin["name"].as_str().ok_or("PLUGIN_NAME")?;
    security::owned_dir(&work)?;
    let package = work.join("plugins").join(name);
    security::owned_dir(&package)?;
    for (rel, hash) in &p.files {
        let bytes = security::read(&PathBuf::from(&p.root).join(rel), 131072)?;
        if digest(&bytes) != *hash {
            return Err("PLUGIN_SOURCE_CHANGED".into());
        }
        let target = package.join(rel);
        security::owned_dir(target.parent().ok_or("PLUGIN_PARENT")?)?;
        security::seal(&target, &bytes)?;
    }
    let market = work.join(".agents/plugins");
    security::owned_dir(&market)?;
    let marketplace = market.join("marketplace.json");
    security::seal(&marketplace,&serde_json::to_vec_pretty(&json!({"name":"skillverse-compat","plugins":[{"name":name,"source":{"source":"local","path":format!("./plugins/{name}")},"policy":{"installation":"AVAILABLE","authentication":"ON_USE"}}]})).map_err(|_|"MARKETPLACE_SERIALIZE")?)?;
    let home = work.join("codex-home");
    security::owned_dir(&home)?;
    let config=toml::to_string(&json!({"marketplaces":{"skillverse-compat":{"source_type":"local","source":work.to_string_lossy()}},"features":{"plugins":true,"hooks":false,"apps":false}})).map_err(|_|"PLUGIN_PROBE_CONFIG")?;
    security::seal(&home.join("config.toml"), config.as_bytes())?;
    let control = AtomicU8::new(0);
    let mut session = super::runtime::Session::connect(&work)?;
    let before = session.rpc(
        "plugin/list",
        json!({"cwds":[work],"forceRefetch":false,"marketplaceKinds":["local"]}),
        &control,
        15000,
    )?;
    let install = session.rpc(
        "plugin/install",
        json!({"pluginName":name,"marketplacePath":marketplace}),
        &control,
        20000,
    )?;
    let installed = session.rpc("plugin/installed", json!({"cwds":[work]}), &control, 15000)?;
    let encoded = installed.to_string();
    if !encoded.contains(&format!("{name}@skillverse-compat")) {
        return Err("CODEX_PLUGIN_NOT_RECOGNIZED".into());
    }
    Ok(
        json!({"scope":"ISOLATED_EXACT_UPSTREAM_SKILL_ONLY_PLUGIN_INSTALL_AND_RECOGNITION","profileId":p.id,"reviewDigest":p.digest(),"repository":p.repository,"commit":p.commit,"work":work,"marketplaceListed":before.to_string().contains(name),"install":install,"installed":installed,"CODEX_PLUGIN_RECOGNIZED":true,"pluginMcpStarted":false,"hooksExecuted":false,"modelCalls":0,"userConfigModified":false}),
    )
}
