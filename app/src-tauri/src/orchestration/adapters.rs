use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
pub type AdapterFuture<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
pub struct Context<'a> {
    pub plan: &'a Plan,
    pub node: &'a Node,
    pub inputs: &'a Value,
    pub workspace: &'a std::path::Path,
    pub control: Arc<AtomicU8>,
    pub progress: tokio::sync::mpsc::UnboundedSender<&'static str>,
}
pub trait CapabilityAdapter: Sync {
    fn supports(&self, id: &str) -> bool;
    fn validate(&self, node: &Node) -> Result<()>;
    fn prepare(&self, node: &Node) -> Result<Value>;
    fn execute<'a>(&'a self, context: Context<'a>) -> AdapterFuture<'a>;
    fn cancel(&self, control: &AtomicU8) {
        control.store(2, Ordering::SeqCst)
    }
    fn collect_evidence(&self, node: &Node, output: &Value) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(&json!({"adapter":node.adapter,"capabilityId":node.capability_id,"capabilityVersion":node.capability_version,"output":output,"scope":"NO_MASTERY_PROMOTION"})).map_err(|_|"EVIDENCE_SERIALIZE".into())
    }
}
struct Builtin;
struct Codex;
struct Skill;
struct Plugin;
struct Mcp;
struct ReviewedSkill;
struct ReviewedPlugin;
struct ReviewedMcp;
static REVIEWED_SKILL: ReviewedSkill = ReviewedSkill;
static REVIEWED_PLUGIN: ReviewedPlugin = ReviewedPlugin;
static REVIEWED_MCP: ReviewedMcp = ReviewedMcp;
static BUILTIN: Builtin = Builtin;
static CODEX: Codex = Codex;
static SKILL: Skill = Skill;
static PLUGIN: Plugin = Plugin;
static MCP: Mcp = Mcp;
pub fn registry() -> [&'static dyn CapabilityAdapter; 8] {
    [
        &BUILTIN,
        &CODEX,
        &SKILL,
        &PLUGIN,
        &MCP,
        &REVIEWED_SKILL,
        &REVIEWED_PLUGIN,
        &REVIEWED_MCP,
    ]
}
pub fn resolve(id: &str) -> Result<&'static dyn CapabilityAdapter> {
    registry()
        .into_iter()
        .find(|a| a.supports(id))
        .ok_or("UNSUPPORTED_ADAPTER".into())
}
pub fn validate(n: &Node) -> Result<()> {
    resolve(&n.adapter)?.validate(n)
}
fn structural(n: &Node) -> Result<()> {
    if n.input_schema != json!({"type":"object"})
        || n.output_schema != json!({"type":"object"})
        || n.risk == "HIGH"
        || !matches!(n.risk.as_str(), "LOW" | "MEDIUM")
        || n.verification_policy != "HASH_BOUND_FIRST_PARTY_OUTPUT_V1"
        || !matches!(
            n.retry_policy.as_str(),
            "SAFE_READ_ONLY" | "OWNED_IDEMPOTENT" | "NEVER"
        )
    {
        return Err("UNSUPPORTED_SCHEMA_RISK_OR_POLICY".into());
    }
    Ok(())
}
impl CapabilityAdapter for Builtin {
    fn supports(&self, id: &str) -> bool {
        [
            "builtin.inspect",
            "builtin.route",
            "builtin.report",
            "builtin.verify",
            "builtin.wait",
        ]
        .contains(&id)
    }
    fn validate(&self, n: &Node) -> Result<()> {
        structural(n)?;
        if n.capability_type != "builtin"
            || n.capability_id.is_some()
            || n.risk
                != if n.adapter == "builtin.report" {
                    "MEDIUM"
                } else {
                    "LOW"
                }
        {
            return Err("BUILTIN_SCOPE_MISMATCH".into());
        }
        if n.adapter == "builtin.report" && n.retry_policy != "OWNED_IDEMPOTENT" {
            return Err("SIDE_EFFECT_RETRY_POLICY".into());
        }
        if n.adapter == "builtin.wait" {
            if n.params.as_object().is_none_or(|p| p.len() != 1)
                || !(1..=15000).contains(&n.params["milliseconds"].as_u64().unwrap_or(0))
            {
                return Err("WAIT_PARAMETERS".into());
            }
        } else if n.params != json!({}) {
            return Err("UNSUPPORTED_EXECUTION_PARAMETERS".into());
        }
        if ["builtin.report", "builtin.verify"].contains(&n.adapter.as_str())
            && n.dependencies.is_empty()
        {
            return Err("MISSING_INPUT_DEPENDENCY".into());
        }
        Ok(())
    }
    fn prepare(&self, n: &Node) -> Result<Value> {
        self.validate(n)?;
        Ok(
            json!({"ready":true,"mode":"LOCAL_DETERMINISTIC","sideEffect":if n.adapter=="builtin.report"{"APP_OWNED_IDEMPOTENT_WRITE"}else{"NONE"},"commandsExecuted":false}),
        )
    }
    fn execute<'a>(&'a self, c: Context<'a>) -> AdapterFuture<'a> {
        Box::pin(async move {
            if c.control.load(Ordering::SeqCst) != 0 {
                return Err("CANCELLED_OR_PAUSED".into());
            }
            match c.node.adapter.as_str() {
                "builtin.inspect" => security::inspect(&c.plan.project_root),
                "builtin.route" => Ok(c.plan.routing.clone()),
                "builtin.wait" => {
                    let ms = c.node.params["milliseconds"]
                        .as_u64()
                        .ok_or("WAIT_PARAMETERS")?;
                    let begin = std::time::Instant::now();
                    while begin.elapsed().as_millis() < u128::from(ms) {
                        if c.control.load(Ordering::SeqCst) != 0 {
                            return Err("CANCELLED_OR_PAUSED".into());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    Ok(json!({"waitedMs":ms,"scope":"FIRST_PARTY_CHECKPOINT_WAIT"}))
                }
                "builtin.report" => {
                    let report = json!({"workflowId":c.plan.workflow_id,"revision":c.plan.revision,"goal":c.plan.goal,"scope":c.plan.scope,"goalVerified":c.plan.goal_verifiable,"inputs":c.inputs,"externalCapabilityExecuted":false,"firstPartySkillProbeExecuted":c.inputs.as_object().is_some_and(|m|m.values().any(|v|v["selectedSkillExecuted"]==true)),"liveModel":"NOT VERIFIED","mastery":"NOT_VERIFIED"});
                    let bytes =
                        serde_json::to_vec_pretty(&report).map_err(|_| "REPORT_SERIALIZE")?;
                    let path = c.workspace.join("report.json");
                    let hash = security::seal(&path, &bytes)?;
                    Ok(
                        json!({"reportPath":path,"sha256":hash,"bytes":bytes.len(),"goalVerified":c.plan.goal_verifiable,"externalCapabilityExecuted":false,"scope":c.plan.scope}),
                    )
                }
                "builtin.verify" => {
                    let source = c
                        .inputs
                        .as_object()
                        .and_then(|o| o.values().find(|v| v.get("reportPath").is_some()))
                        .ok_or("REPORT_INPUT_REQUIRED")?;
                    let path = std::path::PathBuf::from(
                        source["reportPath"].as_str().ok_or("REPORT_PATH")?,
                    );
                    if path != c.workspace.join("report.json") {
                        return Err("ILLEGAL_REPORT_PATH".into());
                    }
                    let bytes = security::read(&path, 262144)?;
                    if digest(&bytes) != source["sha256"].as_str().unwrap_or("") {
                        return Err("REPORT_HASH_MISMATCH".into());
                    }
                    let report: Value =
                        serde_json::from_slice(&bytes).map_err(|_| "INVALID_REPORT_JSON")?;
                    if report["workflowId"] != c.plan.workflow_id
                        || report["revision"] != c.plan.revision
                        || report["externalCapabilityExecuted"] != false
                    {
                        return Err("REPORT_IDENTITY_OR_SCOPE_MISMATCH".into());
                    }
                    Ok(
                        json!({"artifactVerified":true,"sha256":digest(&bytes),"goalVerified":report["goalVerified"],"selectedSkillExecuted":false,"mastery":"NOT_VERIFIED"}),
                    )
                }
                _ => Err("UNSUPPORTED_ADAPTER".into()),
            }
        })
    }
}
impl CapabilityAdapter for Codex {
    fn supports(&self, id: &str) -> bool {
        matches!(id, "codex.fixture_read" | "codex.skill_fixture")
    }
    fn validate(&self, n: &Node) -> Result<()> {
        structural(n)?;
        if n.params != json!({})
            || n.capability_id.is_some()
            || n.capability_type != "builtin"
            || n.risk
                != if n.adapter == "codex.skill_fixture" {
                    "MEDIUM"
                } else {
                    "LOW"
                }
            || n.max_attempts != 1
        {
            return Err("CODEX_FIXED_READ_ONLY_SCOPE".into());
        }
        if n.adapter == "codex.skill_fixture" && n.retry_policy != "NEVER" {
            return Err("INFERENCE_RETRY_FORBIDDEN".into());
        }
        Ok(())
    }
    fn prepare(&self, n: &Node) -> Result<Value> {
        self.validate(n)?;
        if n.adapter == "codex.skill_fixture" {
            return Ok(
                json!({"ready":true,"sideEffect":"ONE_EXISTING_CHATGPT_INFERENCE_NONREPLAYABLE","modelCalls":true,"scope":"Fixed first-party Skill analysis; shell/hooks/apps/plugins disabled; no API key or new login"}),
            );
        }
        Ok(
            json!({"ready":true,"requiresLiveHandshake":true,"sideEffect":"OWNED_RUNTIME_FIXTURE_ONLY","modelCalls":false,"scope":"Fixed first-party read command only; never arbitrary argv"}),
        )
    }
    fn execute<'a>(&'a self, c: Context<'a>) -> AdapterFuture<'a> {
        Box::pin(async move {
            let path = c.workspace.to_path_buf();
            let control = c.control.clone();
            let skill = c.node.adapter == "codex.skill_fixture";
            tauri::async_runtime::spawn_blocking(move || {
                if skill {
                    super::codex_skill::execute(&path, control)
                } else {
                    super::runtime::fixed_read(&path, control)
                }
            })
            .await
            .map_err(|_| "RUNTIME_TASK_EXITED")?
        })
    }
}
macro_rules! unavailable {($t:ty,$id:literal,$kind:literal,$reason:literal)=>{impl CapabilityAdapter for $t{
 fn supports(&self,id:&str)->bool{id==$id}
 fn validate(&self,n:&Node)->Result<()>{structural(n)?;if n.capability_type!=$kind||n.capability_id.is_none()||n.capability_version.is_none()||n.params!=json!({})||n.max_attempts!=1{return Err("EXTERNAL_SCOPE_MISMATCH".into())}Ok(())}
 fn prepare(&self,n:&Node)->Result<Value>{self.validate(n)?;Ok(json!({"ready":false,"status":"UNSUPPORTED_OR_NOT_VERIFIED","reason":$reason,"sideEffect":"NONE","noShellFallback":true}))}
 fn execute<'a>(&'a self,_c:Context<'a>)->AdapterFuture<'a>{Box::pin(async{Err($reason.into())})}
}}}
unavailable!(
    Skill,
    "codex.skill",
    "skill",
    "SKILL_RUNTIME_NOT_VERIFIED_INDEPENDENT_INFERENCE_APPROVAL_REQUIRED"
);
unavailable!(
    Plugin,
    "plugin.bundle",
    "plugin",
    "PLUGIN_BUNDLED_COMPONENT_RUNTIME_OR_ACCOUNT_NOT_VERIFIED"
);
impl CapabilityAdapter for Mcp {
    fn supports(&self, id: &str) -> bool {
        matches!(id, "mcp.tool" | "mcp.fixture")
    }
    fn validate(&self, n: &Node) -> Result<()> {
        structural(n)?;
        if n.adapter == "mcp.fixture" {
            if n.params != json!({})
                || n.capability_id.is_some()
                || n.capability_type != "builtin"
                || n.risk != "LOW"
                || n.max_attempts != 1
            {
                return Err("MCP_FIXED_FIXTURE_SCOPE".into());
            }
        } else if n.capability_type != "mcp"
            || n.capability_id.is_none()
            || n.capability_version.is_none()
            || n.params != json!({})
            || n.max_attempts != 1
        {
            return Err("MCP_SCOPE_MISMATCH".into());
        }
        Ok(())
    }
    fn prepare(&self, n: &Node) -> Result<Value> {
        self.validate(n)?;
        Ok(if n.adapter == "mcp.fixture" {
            json!({"ready":true,"sideEffect":"OWNED_LOOPBACK_FIXTURE_ONLY","scope":"First-party MCP schema/call exercise; no user MCP configuration is read or started"})
        } else {
            json!({"ready":false,"reason":"MCP_NOT_CONNECTED_NO_DISCOVERED_APPROVED_TOOL_SCHEMA","sideEffect":"NONE","noShellFallback":true})
        })
    }
    fn execute<'a>(&'a self, c: Context<'a>) -> AdapterFuture<'a> {
        Box::pin(async move {
            if c.node.adapter == "mcp.fixture" {
                super::mcp::exercise(c.control, 0).await
            } else {
                Err("MCP_NOT_CONNECTED_NO_DISCOVERED_APPROVED_TOOL_SCHEMA".into())
            }
        })
    }
}

macro_rules! reviewed_adapter {($t:ty,$id:literal,$execute:ident)=>{impl CapabilityAdapter for $t {
 fn supports(&self,id:&str)->bool{id==$id}
 fn validate(&self,n:&Node)->Result<()>{structural(n)?;compatibility::validate(n)?;Ok(())}
 fn prepare(&self,n:&Node)->Result<Value>{self.validate(n)?;let p=compatibility::validate(n)?;let checked=compatibility::check(&p);Ok(json!({"ready":checked.is_ok(),"status":if checked.is_ok(){"READY"}else{"BLOCKED"},"reason":checked.err(),"requiresLiveHandshake":true,"scope":compatibility::scope(n),"sideEffect":if p.kind=="mcp"{"PUBLIC_READ_ONLY_MCP"}else{"ONE_EXISTING_CHATGPT_INFERENCE_NONREPLAYABLE"}}))}
 fn execute<'a>(&'a self,c:Context<'a>)->AdapterFuture<'a>{Box::pin(compatibility::$execute(c))}
}}}
reviewed_adapter!(ReviewedSkill, "skill.reviewed", execute_skill);
reviewed_adapter!(ReviewedPlugin, "plugin.reviewed", execute_plugin);
reviewed_adapter!(ReviewedMcp, "mcp.reviewed", execute_mcp);
