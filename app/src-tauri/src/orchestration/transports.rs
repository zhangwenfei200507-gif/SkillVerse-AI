//! MCP stdio framing/lifecycle. This module never chooses or launches a command.
//! A caller must independently review the child, environment and tool policy.
use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin},
    sync::mpsc,
    time::{Duration, Instant},
};
pub(super) const MCP_PREFERRED_VERSION: &str = "2025-11-25";

pub(super) fn negotiated_mcp_version(value: &Value) -> Option<&str> {
    value.as_str().filter(|version| {
        matches!(
            *version,
            "2025-03-26" | "2025-06-18" | MCP_PREFERRED_VERSION
        )
    })
}

pub struct McpStdio {
    child: security::ProcessGuard,
    input: Option<ChildStdin>,
    messages: mpsc::Receiver<Result<Value>>,
    next: u64,
    pub methods: Vec<String>,
}
impl McpStdio {
    pub fn from_reviewed_child(child: Child) -> Result<Self> {
        let mut child = security::ProcessGuard::new(child)?;
        let input = child.stdin.take().ok_or("MCP_STDIO_INPUT")?;
        let output = child.stdout.take().ok_or("MCP_STDIO_OUTPUT")?;
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                match (&mut reader).take(262145).read_until(b'\n', &mut bytes) {
                    Ok(0) => break,
                    Ok(_) if bytes.len() <= 262144 => {
                        if tx
                            .send(
                                serde_json::from_slice(&bytes)
                                    .map_err(|_| "MCP_STDIO_INVALID_JSON".into()),
                            )
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {
                        let _ = tx.send(Err("MCP_STDIO_MESSAGE_LIMIT".into()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            messages: rx,
            next: 0,
            methods: vec![],
        })
    }
    fn send(&mut self, v: Value) -> Result<()> {
        let input = self.input.as_mut().ok_or("MCP_STDIO_CLOSED")?;
        writeln!(input, "{v}")
            .and_then(|_| input.flush())
            .map_err(|_| "MCP_STDIO_DISCONNECTED".into())
    }
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        control: &AtomicU8,
        timeout: Duration,
    ) -> Result<Value> {
        if control.load(Ordering::SeqCst) != 0 {
            return Err("MCP_STDIO_CANCELLED".into());
        }
        if !matches!(method, "initialize" | "tools/list" | "tools/call") {
            return Err("MCP_STDIO_METHOD_NOT_APPROVED".into());
        }
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        self.methods.push(method.into());
        let begin = Instant::now();
        let mut notifications = 0;
        loop {
            if control.load(Ordering::SeqCst) != 0 {
                return Err("MCP_STDIO_CANCELLED".into());
            }
            if begin.elapsed() > timeout {
                return Err("MCP_STDIO_TIMEOUT".into());
            }
            match self.messages.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(v)) => {
                    if v["jsonrpc"] != "2.0" {
                        return Err("MCP_STDIO_VERSION".into());
                    }
                    if v.get("method").is_some() {
                        notifications += 1;
                        if notifications > 100 {
                            return Err("MCP_STDIO_NOTIFICATION_LIMIT".into());
                        }
                        if v.get("id").is_some() {
                            self.send(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Server initiated requests are not authorized"}}))?;
                            return Err("MCP_STDIO_UNAPPROVED_SERVER_REQUEST".into());
                        }
                        continue;
                    }
                    if v["id"] != id {
                        return Err("MCP_STDIO_RESPONSE_ID".into());
                    }
                    if v.get("error").is_some() {
                        return Err("MCP_STDIO_RPC_ERROR".into());
                    }
                    return v
                        .get("result")
                        .cloned()
                        .ok_or("MCP_STDIO_MISSING_RESULT".into());
                }
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("MCP_STDIO_DISCONNECTED".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|_| "MCP_STDIO_PROCESS")?
                        .is_some()
                    {
                        return Err("MCP_STDIO_PROCESS_EXITED".into());
                    }
                }
            }
        }
    }
    pub fn initialize(&mut self, control: &AtomicU8) -> Result<Value> {
        let result=self.request("initialize",json!({"protocolVersion":MCP_PREFERRED_VERSION,"capabilities":{},"clientInfo":{"name":"skillverse-ai","version":"9.5"}}),control,Duration::from_secs(10))?;
        if negotiated_mcp_version(&result["protocolVersion"]).is_none() {
            return Err("MCP_STDIO_PROTOCOL_UNSUPPORTED".into());
        }
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        self.methods.push("notifications/initialized".into());
        Ok(result)
    }
    pub fn close(mut self) -> Result<Value> {
        self.input.take();
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| "MCP_STDIO_PROCESS")? {
                return Ok(
                    json!({"naturalExit":true,"success":status.success(),"methods":self.methods}),
                );
            }
            if start.elapsed() > Duration::from_secs(2) {
                return Err("MCP_STDIO_EOF_EXIT_TIMEOUT_OWNED_TREE_WILL_BE_STOPPED".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
