use super::{Installer, config, files, types::*};
use crate::github::http::RequestContext;
use serde_json::{Value, json};
use std::time::Duration;

const VERSION: &str = "2025-11-25";
const LIMIT: usize = 256 * 1024;

// Deliberately a bounded compatibility probe, not a general MCP execution client.
// Untrusted instructions, schemas and response bodies never enter logs or prompts.
async fn message(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    session: Option<&str>,
    version: &str,
    body: Value,
    context: &RequestContext,
) -> Result<(Value, Option<String>)> {
    if context.cancel.is_cancelled() {
        return Err(fail(
            "CANCELLED",
            "The authorized connection test was cancelled.",
        ));
    }
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", version)
        .json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(session) = session {
        request = request.header("Mcp-Session-Id", session);
    }
    let mut response = request.send().await.map_err(|_| {
        fail(
            "CONNECTION_FAILED",
            "The approved MCP endpoint could not be reached within the bounded test.",
        )
    })?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(fail(
            "AUTH_REQUIRED",
            "The MCP endpoint requires separate account authorization.",
        ));
    }
    if !response.status().is_success() {
        return Err(fail(
            "CONNECTION_FAILED",
            "The MCP endpoint rejected the authorized test. Response content is not logged.",
        ));
    }
    let session = response
        .headers()
        .get("Mcp-Session-Id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 256 && value.bytes().all(|b| (0x21..=0x7e).contains(&b)))
        .map(str::to_owned);
    if body.get("id").is_none() {
        return Ok((Value::Null, session));
    }
    if response
        .content_length()
        .is_some_and(|size| size > LIMIT as u64)
    {
        return Err(fail(
            "PROBE_LIMIT",
            "MCP test response exceeded its byte limit.",
        ));
    }
    let streaming = response
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        fail(
            "CONNECTION_FAILED",
            "The bounded MCP response was interrupted.",
        )
    })? {
        if context.cancel.is_cancelled() {
            return Err(fail(
                "CANCELLED",
                "The authorized connection test was cancelled.",
            ));
        }
        if bytes.len() + chunk.len() > LIMIT {
            return Err(fail(
                "PROBE_LIMIT",
                "MCP test response exceeded its byte limit.",
            ));
        }
        bytes.extend_from_slice(&chunk);
        if streaming {
            let raw = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            for event in raw.split("\n\n").take(raw.matches("\n\n").count()) {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Ok(value) = serde_json::from_str::<Value>(&data) {
                    if value["id"] == body["id"] {
                        return rpc_result(value, session);
                    }
                }
            }
        }
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        fail(
            "PROTOCOL",
            "The endpoint did not return a supported bounded MCP response.",
        )
    })?;
    if value["id"] != body["id"] {
        return Err(fail(
            "PROTOCOL",
            "MCP response did not match the authorized request.",
        ));
    }
    rpc_result(value, session)
}
fn rpc_result(value: Value, session: Option<String>) -> Result<(Value, Option<String>)> {
    if value["jsonrpc"] != "2.0" || value.get("error").is_some() || value.get("result").is_none() {
        return Err(fail(
            "PROTOCOL",
            "The endpoint returned an MCP protocol error. Server content is not logged.",
        ));
    }
    Ok((value["result"].clone(), session))
}
async fn connect(value: &Value, context: &RequestContext) -> Result<Value> {
    let url = value["url"].as_str().ok_or_else(|| {
        fail(
            "RUNTIME_NOT_SUPPORTED",
            "No stdio service is launched by SkillVerse.",
        )
    })?;
    // Revalidate the endpoint independently of a saved plan or edited database.
    config::mcp_value(&McpDraft {
        name: "probe".into(),
        transport: "http".into(),
        command: None,
        args: vec![],
        url: Some(url.into()),
        environment_names: vec![],
        bearer_token_environment: value["bearer_token_env_var"].as_str().map(str::to_owned),
        auth_required: false,
    })?;
    let token = if let Some(name) = value["bearer_token_env_var"].as_str() {
        if !url.starts_with("https://") {
            return Err(fail(
                "AUTH_REQUIRED",
                "Credentials may be sent only over the explicitly approved HTTPS endpoint.",
            ));
        }
        Some(
            std::env::var(name)
                .ok()
                .filter(|token| !token.is_empty())
                .ok_or_else(|| {
                    fail(
                        "AUTH_REQUIRED",
                        "The required credential environment variable is unavailable.",
                    )
                })?,
        )
    } else {
        None
    };
    let client = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .build()
        .map_err(|_| fail("CONNECTION_FAILED", "The HTTP test client is unavailable."))?;
    let (init, session) = message(&client, url, token.as_deref(), None, VERSION, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"SkillVerse explicit probe","version":"0.1.0"}}}), context).await?;
    let version = init["protocolVersion"].as_str().filter(|v| ["2025-11-25","2025-06-18","2025-03-26"].contains(v)).ok_or_else(|| fail("PROTOCOL_VERSION", "This bounded probe supports MCP 2025 compatibility mode. No modern-protocol success is inferred."))?;
    let result = async {
        message(&client,url,token.as_deref(),session.as_deref(),version,json!({"jsonrpc":"2.0","method":"notifications/initialized"}),context).await?;
        if init["capabilities"].get("tools").is_none() { return Ok(json!({"connected":true,"toolsDiscovered":false,"toolCount":0,"protocolVersion":version})); }
        let (tools, _) = message(&client,url,token.as_deref(),session.as_deref(),version,json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),context).await?;
        let list = tools["tools"].as_array().ok_or_else(|| fail("PROTOCOL", "The MCP tools list is invalid."))?;
        if list.iter().any(|tool| tool["name"].as_str().is_none() || tool["inputSchema"]["type"] != "object") { return Err(fail("PROTOCOL", "An MCP tool declaration is invalid.")); }
        Ok(json!({"connected":true,"toolsDiscovered":true,"toolCount":list.len(),"moreToolsAvailable":tools.get("nextCursor").is_some(),"protocolVersion":version}))
    }.await;
    if let Some(session) = session.as_ref() {
        let mut request = client
            .delete(url)
            .header("Mcp-Session-Id", session)
            .header("MCP-Protocol-Version", version);
        if let Some(token) = token.as_ref() {
            request = request.bearer_auth(token);
        }
        let _ = request.send().await;
    }
    result
}
impl Installer {
    pub(super) async fn probe(&self, op: &Operation, context: &RequestContext) -> Result<()> {
        let mut record = op
            .plan
            .prior_record
            .clone()
            .ok_or_else(|| fail("OWNERSHIP", "The approved managed MCP no longer exists."))?;
        for change in &op.plan.changes {
            if files::image(&files::read(&self.profile, &change.target)?)? != change.before {
                return Err(fail(
                    "STALE_PLAN",
                    "MCP configuration changed after preview; review a new test.",
                ));
            }
        }
        let owned = record
            .patches
            .iter()
            .find(|patch| patch.patch.section == "mcp_servers")
            .ok_or_else(|| fail("CONFIG", "The MCP declaration is missing."))?;
        let value = owned
            .patch
            .value
            .as_ref()
            .ok_or_else(|| fail("CONFIG", "The MCP declaration is missing."))?;
        self.db.install_event(&op.id,"VERIFYING",&json!({"authorizedNetworkProbe":true,"toolsCalled":false,"subprocessStarted":false})).await?;
        let result = connect(value, context).await;
        record.verification.retain(|v| {
            ![
                "CONNECTION_NOT_TESTED",
                "CONNECTED",
                "TOOLS_DISCOVERED",
                "CONNECTION_FAILED",
                "FAILED",
                "AUTH_REQUIRED",
            ]
            .contains(&v.as_str())
        });
        match &result {
            Ok(details) => {
                record.verification.push("CONNECTED".into());
                if details["toolsDiscovered"] == true {
                    record.verification.push("TOOLS_DISCOVERED".into());
                }
            }
            Err(error) => {
                record.verification.push(
                    if error.kind == "AUTH_REQUIRED" {
                        "AUTH_REQUIRED"
                    } else {
                        "FAILED"
                    }
                    .into(),
                );
                record.verification.push("CONNECTION_FAILED".into());
            }
        }
        // A connection observation does not change file ownership or the last installation operation.
        let updated = sqlx::query("UPDATE managed_installations SET record_json=? WHERE id=? AND profile_id=? AND current_operation_id=? AND state!='UNINSTALLED'").bind(serde_json::to_string(&record).map_err(|_| fail("JOURNAL","Invalid managed record."))?).bind(&record.id).bind(&self.profile.id).bind(&record.current_operation_id).execute(&self.db.pool).await.map_err(|_| Error::database())?;
        if updated.rows_affected() != 1 {
            return Err(fail(
                "STALE_PLAN",
                "The managed MCP changed during the test.",
            ));
        }
        let details = result?;
        self.db.install_event(&op.id, "COMPLETED", &details).await?;
        self.db.install_unlock(&op.id).await?;
        Ok(())
    }
}
