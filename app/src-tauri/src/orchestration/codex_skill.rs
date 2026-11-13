//! Bounded first-party Skill acceptance via official CLI, not a universal Skill runner.
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
fn command() -> Result<(Command, security::CodexExecutable)> {
    let exe = security::codex_executable()?;
    let mut command = Command::new(exe.path());
    command.env_clear();
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "CODEX_HOME",
    ] {
        if let Some(v) = std::env::var_os(key) {
            command.env(key, v);
        }
    }
    command
        .env("TEMP", "D:/PersonalProject_DevTools/Temp")
        .env("TMP", "D:/PersonalProject_DevTools/Temp");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok((command, exe))
}
pub(super) fn chatgpt_login() -> Result<()> {
    let (mut command, _verified_executable) = command()?;
    let output = command
        .args(["login", "status"])
        .output()
        .map_err(|_| "CODEX_AUTH_STATUS_UNAVAILABLE")?;
    let raw = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    if !output.status.success() || !raw.contains("chatgpt") || raw.contains("api key") {
        return Err("EXISTING_CHATGPT_LOGIN_REQUIRED_NO_NEW_LOGIN_OR_API_KEY".into());
    }
    Ok(())
}
pub(super) fn execute(workspace: &std::path::Path, control: Arc<AtomicU8>) -> Result<Value> {
    chatgpt_login()?;
    let work = workspace.join("skill-acceptance");
    let skill = work.join(".agents/skills/skillverse-readonly-acceptance");
    security::owned_dir(&skill)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let code = "export function total(items) { return items.reduce((a, b) => a + b); }\n";
    let fixture_hash = security::seal(&work.join("sample.js"), code.as_bytes())?;
    let instructions = format!(
        "---\nname: skillverse-readonly-acceptance\ndescription: Analyze supplied first-party JavaScript source without tools or changes.\n---\n# Read-only code analysis contract\nDo not call tools, shell, network or accounts. Return ONLY JSON: skill_nonce must be {nonce}, issue must be REDUCE_WITHOUT_INITIAL_VALUE when reduce has no initial accumulator, empty_input_throws must indicate the empty-array behavior, and fix must contain a corrected JavaScript expression. Never claim JavaScript execution; this is source analysis.\n"
    );
    let skill_hash = security::seal(&skill.join("SKILL.md"), instructions.as_bytes())?;
    let (mut cmd, verified_executable) = command()?;
    cmd.args([
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--json",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "-C",
    ])
    .arg(&work);
    for feature in [
        "shell_tool",
        "hooks",
        "apps",
        "plugins",
        "skill_mcp_dependency_install",
        "plugin_discovery_prefetch",
        "shell_snapshot",
    ] {
        cmd.args(["--disable", feature]);
    }
    cmd.args([
        "-c",
        "web_search=\"disabled\"",
        "-c",
        "model_reasoning_effort=\"low\"",
        "-",
    ])
    .current_dir(&work)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let mut child =
        security::ProcessGuard::new(cmd.spawn().map_err(|_| "CODEX_SKILL_PROCESS_START")?)?;
    drop(verified_executable);
    let mut input = child.stdin.take().ok_or("CODEX_STDIN")?;
    writeln!(input,"$skillverse-readonly-acceptance Analyze the following JavaScript text without using any tools. Follow the Skill output contract. Source:\n{code}").map_err(|_|"CODEX_INPUT")?;
    drop(input);
    let stdout = child.stdout.take().ok_or("CODEX_STDOUT")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut total = 0;
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    total += line.len();
                    if total > 262144 {
                        let _ = tx.send(Err("CODEX_EVENT_SIZE_LIMIT"));
                        break;
                    }
                    let value =
                        serde_json::from_str::<Value>(&line).map_err(|_| "CODEX_INVALID_EVENT");
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Err("CODEX_STREAM_ERROR"));
                    break;
                }
            }
        }
    });
    let begin = Instant::now();
    let mut methods = Vec::new();
    let mut completed = false;
    let mut final_text = String::new();
    let mut failed = None;
    let mut usage = Value::Null;
    loop {
        if control.load(Ordering::SeqCst) != 0 {
            failed = Some("CODEX_SKILL_CANCELLED_NO_AUTOMATIC_REPLAY");
            break;
        }
        if begin.elapsed() > Duration::from_secs(110) {
            failed = Some("CODEX_SKILL_TIMEOUT_NO_AUTOMATIC_REPLAY");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(event)) => {
                let kind = event["type"].as_str().unwrap_or("UNKNOWN");
                methods.push(kind.to_string());
                if methods.len() > 300 {
                    failed = Some("CODEX_EVENT_LIMIT");
                    break;
                }
                let item = event["item"]["type"].as_str().unwrap_or("");
                if [
                    "command_execution",
                    "mcp_tool_call",
                    "file_change",
                    "web_search",
                ]
                .contains(&item)
                {
                    failed = Some("CODEX_UNAPPROVED_TOOL_EVENT");
                    break;
                }
                if kind == "turn.failed" || kind == "error" {
                    failed = Some("CODEX_MODEL_TURN_FAILED");
                    break;
                }
                if kind == "turn.completed" {
                    completed = true;
                    usage = event["usage"].clone();
                }
                if kind == "item.completed" && item == "agent_message" {
                    final_text = event["item"]["text"].as_str().unwrap_or("").to_string();
                }
            }
            Ok(Err(e)) => {
                failed = Some(e);
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child
                    .try_wait()
                    .map_err(|_| "CODEX_PROCESS_STATUS")?
                    .is_some()
                {
                    continue;
                }
            }
        }
    }
    if failed.is_some() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failed.unwrap().into());
    }
    let status = child.wait().map_err(|_| "CODEX_PROCESS_STATUS")?;
    let output: Value = final_text
        .find('{')
        .zip(final_text.rfind('}'))
        .and_then(|(a, b)| serde_json::from_str(&final_text[a..=b]).ok())
        .ok_or("CODEX_SKILL_OUTPUT_SCHEMA")?;
    let fix = output["fix"].as_str().unwrap_or("").replace(' ', "");
    if !status.success()
        || !completed
        || output["skill_nonce"] != nonce
        || output["issue"] != "REDUCE_WITHOUT_INITIAL_VALUE"
        || output["empty_input_throws"] != true
        || !fix.contains(",0)")
        || digest(&security::read(&work.join("sample.js"), 8192)?) != fixture_hash
        || digest(&security::read(&skill.join("SKILL.md"), 8192)?) != skill_hash
    {
        return Err("CODEX_SKILL_RESULT_NOT_INDEPENDENTLY_VERIFIED".into());
    }
    Ok(
        json!({"scope":"REAL_FIRST_PARTY_SKILL_SOURCE_ANALYSIS_ONLY_NOT_GENERIC_SKILL_OR_PLUGIN_EXECUTION","runtime":"OFFICIAL_CODEX_CLI_EPHEMERAL","authentication":"EXISTING_CHATGPT_LOGIN_NO_NEW_ACCOUNT_OR_API_KEY","selectedSkillExecuted":true,"skillRecognizedAndContractApplied":true,"skillNonceOnlyInSkillSource":true,"actualModelTurnCompleted":true,"fixtureUnchanged":true,"fixtureSha256":fixture_hash,"skillSha256":skill_hash,"sourceIssue":"REDUCE_WITHOUT_INITIAL_VALUE","outputDigest":digest(output.to_string().as_bytes()),"prohibitedToolEvents":0,"eventTypes":methods,"usage":usage,"modelCalls":1,"userMastery":"NOT_VERIFIED"}),
    )
}
