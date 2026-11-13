//! Owned stdio app-server. No daemon attachment, global configuration or inference.
//! Only the fixed first-party fixture read is executable in this adapter.
use super::*;
use base64::Engine;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
pub struct Session {
    child: security::ProcessGuard,
    input: Option<ChildStdin>,
    messages: mpsc::Receiver<Result<Value>>,
    next: u64,
    pending: Vec<Value>,
    pub methods: Vec<String>,
    pub stdout: Vec<u8>,
    pub tool_ceiling: Option<Value>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeScope {
    LegacyIsolated,
    LegacyAuthenticated,
    ApprovedFirstParty,
}

fn bound_decline(msg: &Value, thread: &str, turn: &str) -> Option<Value> {
    let params = &msg["params"];
    let id = msg.get("id")?;
    (matches!(
        msg["method"].as_str(),
        Some("item/commandExecution/requestApproval" | "item/fileChange/requestApproval")
    ) && (id.is_string() || id.is_i64())
        && params["threadId"] == thread
        && params["turnId"] == turn
        && params["itemId"].as_str().is_some_and(|item| {
            !item.is_empty() && item.len() <= 256 && !item.chars().any(char::is_control)
        }))
    .then(|| json!({"id":id,"result":{"decision":"decline"}}))
}
pub(super) fn restrict_model_catalog(mut catalog: Value) -> Result<Value> {
    let models = catalog["models"]
        .as_array_mut()
        .filter(|xs| !xs.is_empty() && xs.len() <= 100)
        .ok_or("MODEL_TOOL_CEILING_METADATA_UNAVAILABLE")?;
    for model in models {
        if !model["slug"].is_string() {
            return Err("MODEL_TOOL_CEILING_METADATA_INVALID".into());
        }
        model["shell_type"] = json!("disabled");
        model["apply_patch_tool_type"] = Value::Null;
        model["experimental_supported_tools"] = json!([]);
        model["supports_search_tool"] = json!(false);
        model["node_repl_disabled"] = json!(true);
        model["tool_mode"] = json!("direct");
        model["multi_agent_version"] = Value::Null;
    }
    Ok(catalog)
}
impl Session {
    pub fn connect(workspace: &std::path::Path) -> Result<Self> {
        Self::connect_mode(workspace, RuntimeScope::LegacyIsolated)
    }
    pub(super) fn connect_authenticated(workspace: &std::path::Path) -> Result<Self> {
        Self::connect_mode(workspace, RuntimeScope::LegacyAuthenticated)
    }
    fn connect_mode(workspace: &std::path::Path, scope: RuntimeScope) -> Result<Self> {
        let authenticated = scope == RuntimeScope::LegacyAuthenticated;
        let first_party = scope == RuntimeScope::ApprovedFirstParty;
        security::owned_dir(workspace)?;
        let home = if authenticated {
            std::env::var_os("CODEX_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var_os("USERPROFILE")
                        .map(|p| std::path::PathBuf::from(p).join(".codex"))
                })
                .ok_or("CODEX_HOME_REQUIRED")?
        } else if first_party {
            workspace.join(format!("codex-home-{}", uuid::Uuid::new_v4()))
        } else {
            workspace.join("codex-home")
        };
        if !authenticated {
            security::owned_dir(&home)?;
        }
        let exe = if first_party {
            security::first_party_codex_executable()?
        } else {
            security::codex_executable()?
        };
        let mut command = Command::new(exe.path());
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(workspace)
            .env_clear();
        for key in ["SystemRoot", "WINDIR"] {
            if let Some(v) = std::env::var_os(key) {
                command.env(key, v);
            }
        }
        if first_party {
            let local = home.join("AppData/Local");
            let roaming = home.join("AppData/Roaming");
            security::owned_dir(&local)?;
            security::owned_dir(&roaming)?;
            let system32 = std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .ok_or("WINDOWS_SYSTEM_ROOT_REQUIRED")?
                .join("System32");
            let path =
                std::env::join_paths([system32.clone(), system32.join("WindowsPowerShell/v1.0")])
                    .map_err(|_| "WINDOWS_SYSTEM_PATH")?;
            command
                .env("PATH", path)
                .env("USERPROFILE", &home)
                .env("LOCALAPPDATA", local)
                .env("APPDATA", roaming);
            command.args([
                "--disable",
                "apps",
                "--disable",
                "plugins",
                "--disable",
                "hooks",
                "--disable",
                "plugin_discovery_prefetch",
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"read-only\"",
            ]);
        } else {
            for key in ["PATH", "USERPROFILE", "LOCALAPPDATA", "APPDATA"] {
                if let Some(v) = std::env::var_os(key) {
                    command.env(key, v);
                }
            }
        }
        let mut tool_ceiling = None;
        if authenticated {
            for feature in [
                "shell_tool",
                "hooks",
                "apps",
                "plugins",
                "skill_mcp_dependency_install",
                "plugin_discovery_prefetch",
                "shell_snapshot",
                "browser_use",
                "browser_use_external",
                "browser_use_full_cdp_access",
                "computer_use",
                "image_generation",
                "view_image",
                "multi_agent",
                "multi_agent_v2",
                "workspace_dependencies",
                "code_mode",
                "code_mode_host",
                "sleep_tool",
                "goals",
                "skill_search",
                "tool_suggest",
                "standalone_web_search",
                "current_time_reminder",
                "request_permissions_tool",
                "token_budget",
                "deferred_executor",
                "artifact",
            ] {
                command.args(["--disable", feature]);
            }
            command.args([
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "tools.update_plan.enabled=false",
            ]);
            let cache = security::read(&home.join("models_cache.json"), 2000000)
                .map_err(|_| "MODEL_TOOL_CEILING_METADATA_UNAVAILABLE")?;
            let restricted = restrict_model_catalog(
                serde_json::from_slice(&cache)
                    .map_err(|_| "MODEL_TOOL_CEILING_METADATA_INVALID")?,
            )?;
            let metadata =
                serde_json::to_vec(&restricted).map_err(|_| "MODEL_TOOL_CEILING_SERIALIZE")?;
            let ids: Vec<_> = restricted["models"]
                .as_array()
                .ok_or("MODEL_METADATA")?
                .iter()
                .filter_map(|m| m["slug"].as_str())
                .collect();
            let path = workspace.join("restricted-model-catalog.json");
            let hash = security::seal(&path, &metadata)?;
            command.args([
                "-c",
                &format!("model_catalog_json={}", json!(path.to_string_lossy())),
            ]);
            tool_ceiling = Some(
                json!({"policy":"INSTRUCTION_ONLY_TOOL_CONSTRUCTION_CEILING_V1","metadataSha256":hash,"modelIdsPreserved":true,"applyPatchRemoved":true,"codeModeRemoved":true,"experimentalToolsRemoved":true,"viewImageBrowserComputerImagegenAndAgentsDisabled":true,"globalCatalogModified":false}),
            );
            if let Some(proof) = tool_ceiling.as_mut() {
                proof["reviewedModelIds"] = json!(ids);
                proof["runtimeBinarySha256"] = json!(security::REVIEWED_CODEX_SHA256);
                proof["existingMcpEntriesMaskedInChild"] = json!(true);
            }
            let config = home.join("config.toml");
            if config.is_file() {
                let raw = security::read(&config, 262144)?;
                let raw = String::from_utf8(raw).map_err(|_| "CODEX_CONFIG_UTF8")?;
                let cfg: toml::Value = toml::from_str(&raw).map_err(|_| "CODEX_CONFIG_PARSE")?;
                if let Some(servers) = cfg.get("mcp_servers").and_then(toml::Value::as_table) {
                    for name in servers.keys() {
                        if name.is_empty()
                            || !name
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                        {
                            return Err("MCP_OVERRIDE_NAME_UNSUPPORTED".into());
                        }
                        let inert = format!(
                            "mcp_servers.{name}={{command={},args=[\"--version\"],enabled=false,env={{}},env_vars=[]}}",
                            json!(exe.path().to_string_lossy())
                        );
                        command.args(["-c", &inert]);
                    }
                }
            }
        }
        command
            .env("CODEX_HOME", home)
            .env("TEMP", "D:/PersonalProject_DevTools/Temp")
            .env("TMP", "D:/PersonalProject_DevTools/Temp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = security::ProcessGuard::new(
            command.spawn().map_err(|_| "CODEX_PROCESS_START_FAILED")?,
        )?;
        drop(exe);
        let input = child.stdin.take().ok_or("RUNTIME_STDIN")?;
        let output = child.stdout.take().ok_or("RUNTIME_STDOUT")?;
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                match reader.by_ref().take(1048577).read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) if line.len() <= 1048576 => {
                        let msg = serde_json::from_slice(&line)
                            .map_err(|_| "RUNTIME_PROTOCOL_INVALID_JSON".into());
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                    _ => {
                        let _ = tx.send(Err("RUNTIME_PROTOCOL_SIZE_LIMIT".into()));
                        break;
                    }
                }
            }
        });
        let mut s = Self {
            child,
            input: Some(input),
            messages: rx,
            next: 0,
            pending: Vec::new(),
            methods: Vec::new(),
            stdout: Vec::new(),
            tool_ceiling,
        };
        let control = AtomicU8::new(0);
        s.rpc("initialize",json!({"clientInfo":{"name":"skillverse-ai","title":"SkillVerse AI","version":"0.1.0"},"capabilities":{"experimentalApi":true}}),&control,15000)?;
        s.send(json!({"method":"initialized","params":{}}))?;
        Ok(s)
    }
    pub(super) fn wait_turn(
        &mut self,
        thread: &str,
        turn: &str,
        control: &AtomicU8,
    ) -> Result<Value> {
        let begin = Instant::now();
        let mut text = String::new();
        let mut event_types = Vec::new();
        let mut count = 0;
        loop {
            if control.load(Ordering::SeqCst) != 0 || begin.elapsed() > Duration::from_secs(105) {
                let _=self.send(json!({"id":999999,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}}));
                return Err("CODEX_TURN_INTERRUPTED_NO_AUTOMATIC_REPLAY".into());
            }
            let msg = if !self.pending.is_empty() {
                self.pending.remove(0)
            } else {
                match self.messages.recv_timeout(Duration::from_millis(25)) {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Err(e),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("CODEX_TURN_DISCONNECTED".into());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                }
            };
            count += 1;
            if count > 1000 {
                return Err("CODEX_TURN_EVENT_LIMIT".into());
            }
            if msg.get("method").is_some() && msg.get("id").is_some() {
                self.send(bound_decline(&msg, thread, turn).unwrap_or_else(|| {
                    json!({"id":msg["id"],"error":{"code":-32000,"message":"Unapproved tool request denied"}})
                }))?;
                return Err("CODEX_UNAPPROVED_REQUEST_DENIED".into());
            }
            let method = msg["method"].as_str().unwrap_or("");
            if !method.is_empty() {
                event_types.push(method.to_string());
            }
            let item = &msg["params"]["item"];
            if matches!(
                item["type"].as_str(),
                Some(
                    "commandExecution"
                        | "mcpToolCall"
                        | "fileChange"
                        | "webSearch"
                        | "dynamicToolCall"
                )
            ) {
                return Err("CODEX_UNAPPROVED_TOOL_EVENT".into());
            }
            if method == "item/completed" && item["type"] == "agentMessage" {
                text = item["text"].as_str().unwrap_or("").to_string();
            }
            if method == "turn/completed"
                && msg["params"]["threadId"] == thread
                && msg["params"]["turn"]["id"] == turn
            {
                if msg["params"]["turn"]["status"] != "completed" {
                    return Err("CODEX_MODEL_TURN_FAILED".into());
                }
                if text.is_empty() || text.len() > 65536 {
                    return Err("CODEX_OUTPUT_MISSING_OR_SIZE".into());
                }
                return Ok(
                    json!({"text":text,"events":event_types,"threadId":thread,"turnId":turn,"turnCompleted":true,"unapprovedToolEvents":0}),
                );
            }
        }
    }
    fn send(&mut self, msg: Value) -> Result<()> {
        let input = self.input.as_mut().ok_or("RUNTIME_CLOSED")?;
        writeln!(input, "{msg}")
            .and_then(|_| input.flush())
            .map_err(|_| "RUNTIME_DISCONNECTED".into())
    }
    pub fn rpc(
        &mut self,
        method: &str,
        params: Value,
        control: &AtomicU8,
        timeout: u64,
    ) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let begin = Instant::now();
        loop {
            if control.load(Ordering::SeqCst) != 0 {
                let _=self.send(json!({"id":id+1000000,"method":"command/exec/terminate","params":{"processId":"skillverse-fixed-read"}}));
                return Err("RUNTIME_CANCELLED".into());
            }
            if begin.elapsed().as_millis() > u128::from(timeout) {
                let _=self.send(json!({"id":id+1000000,"method":"command/exec/terminate","params":{"processId":"skillverse-fixed-read"}}));
                return Err("RUNTIME_TIMEOUT".into());
            }
            match self.messages.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok(msg)) => {
                    if msg["id"] == id && msg.get("error").is_some() {
                        let code = msg["error"]["code"]
                            .as_i64()
                            .ok_or("RUNTIME_PROTOCOL_INVALID_ERROR")?;
                        return Err(format!("RUNTIME_RPC_ERROR_{code}"));
                    }
                    if msg["id"] == id && msg.get("result").is_some() {
                        return Ok(msg["result"].clone());
                    }
                    if let Some(name) = msg["method"].as_str() {
                        if self.methods.len() >= 512 {
                            return Err("RUNTIME_EVENT_LIMIT".into());
                        }
                        self.methods.push(name.into());
                        self.pending.push(msg.clone());
                        if msg.get("id").is_some() {
                            self.send(json!({"id":msg["id"],"error":{"code":-32000,"message":"SkillVerse denied an unapproved server request"}}))?;
                            return Err("RUNTIME_UNAPPROVED_REQUEST_DENIED".into());
                        }
                        if name == "command/exec/outputDelta" && msg["params"]["stream"] == "stdout"
                        {
                            if msg["params"]["processId"] != "skillverse-fixed-read"
                                || msg["params"]["capReached"] == true
                            {
                                return Err("RUNTIME_OUTPUT_SCOPE_OR_LIMIT".into());
                            }
                            let bytes = base64::engine::general_purpose::STANDARD
                                .decode(msg["params"]["deltaBase64"].as_str().unwrap_or(""))
                                .map_err(|_| "RUNTIME_INVALID_OUTPUT")?;
                            self.stdout.extend(bytes);
                            if self.stdout.len() > 8192 {
                                return Err("RUNTIME_OUTPUT_LIMIT".into());
                            }
                        }
                    }
                }
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("RUNTIME_DISCONNECTED".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|_| "RUNTIME_PROCESS_STATUS")?
                        .is_some()
                    {
                        return Err("RUNTIME_EXITED".into());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;

    #[test]
    fn runtime_decline_matches_active_thread_turn_and_concrete_item() {
        let request = json!({
            "id":73,"method":"item/commandExecution/requestApproval",
            "params":{"threadId":"thread-1","turnId":"turn-2","itemId":"item-3"}
        });
        assert_eq!(
            bound_decline(&request, "thread-1", "turn-2"),
            Some(json!({"id":73,"result":{"decision":"decline"}}))
        );
        assert!(bound_decline(&request, "thread-2", "turn-2").is_none());
        assert!(bound_decline(&request, "thread-1", "turn-3").is_none());
        let mut invalid = request.clone();
        invalid["params"]["itemId"] = json!("");
        assert!(bound_decline(&invalid, "thread-1", "turn-2").is_none());
        invalid = request;
        invalid["method"] = json!("mcpServer/elicitation/request");
        assert!(bound_decline(&invalid, "thread-1", "turn-2").is_none());
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        let begin = Instant::now();
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if begin.elapsed() > Duration::from_secs(2) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
pub fn probe() -> Result<Value> {
    let work = security::root()
        .join("runtime-probes")
        .join(uuid::Uuid::new_v4().to_string());
    let mut s = Session::connect_mode(&work, RuntimeScope::ApprovedFirstParty)?;
    let control = AtomicU8::new(0);
    let skills = s.rpc(
        "skills/list",
        json!({"cwds":[work],"forceReload":true}),
        &control,
        15000,
    )?;
    let thread = s.rpc(
        "thread/start",
        json!({"cwd":work,"sandbox":"read-only","approvalPolicy":"never","ephemeral":true}),
        &control,
        15000,
    )?;
    Ok(
        json!({"status":"PROTOCOL_CONNECTED","transport":"stdio","processOwned":true,"isolatedHome":true,"globalConfigModified":false,"trustScope":"APPROVED_FIRST_PARTY_ISOLATED","binarySha256":security::FIRST_PARTY_CODEX_SHA256,"signatureVerified":true,"ephemeralThreadCreated":thread["thread"]["id"].is_string(),"discoveredSkills":skills["data"][0]["skills"].as_array().map(Vec::len),"skillExecution":"NOT VERIFIED","pluginExecution":"NOT VERIFIED","mcpRuntime":"NOT VERIFIED","inference":"NOT VERIFIED","account":"NOT_CONNECTED_TO_ISOLATED_RUNTIME","methods":s.methods}),
    )
}
pub fn fixed_read(workspace: &std::path::Path, control: Arc<AtomicU8>) -> Result<Value> {
    let work = workspace.join("runtime");
    security::owned_dir(&work)?;
    let content = b"SkillVerse controlled read-only fixture\nvalue\n13\n21\n34\n";
    let file = work.join("input.txt");
    let before = security::seal(&file, content)?;
    let mut s = Session::connect_mode(&work, RuntimeScope::ApprovedFirstParty)?;
    let result=s.rpc("command/exec",json!({"command":["powershell.exe","-NoProfile","-NonInteractive","-Command","Get-Content -LiteralPath 'input.txt'"],"cwd":work,"sandboxPolicy":{"type":"readOnly","networkAccess":false},"processId":"skillverse-fixed-read","streamStdoutStderr":true,"timeoutMs":10000,"outputBytesCap":8192}),&control,15000)?;
    let output = String::from_utf8(s.stdout.clone())
        .map_err(|_| "RUNTIME_OUTPUT_ENCODING")?
        .replace('\r', "");
    if result["exitCode"] != 0
        || output != String::from_utf8_lossy(content)
        || digest(&security::read(&file, 8192)?) != before
    {
        return Err("REAL_RUNTIME_FIXTURE_VERIFICATION_FAILED".into());
    }
    Ok(
        json!({"runtime":"ACTUAL_CODEX_APP_SERVER_COMMAND_EXEC","exitCode":0,"fixtureSha256":before,"outputSha256":digest(output.as_bytes()),"fixtureUnchanged":true,"streamedOutput":true,"eventMethods":s.methods,"selectedSkillExecuted":false,"modelCalls":0,"binarySha256":security::FIRST_PARTY_CODEX_SHA256,"scope":"FIXED_FIRST_PARTY_READ_ONLY_COMMAND_NOT_SKILL_ACTIVATION"}),
    )
}
