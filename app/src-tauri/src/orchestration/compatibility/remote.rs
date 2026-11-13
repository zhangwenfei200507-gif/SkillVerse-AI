use super::*;
use crate::orchestration::transports::{MCP_PREFERRED_VERSION, negotiated_mcp_version};
use std::time::Duration;
pub(super) fn public_arguments(tool: &str, goal: &str) -> Result<Value> {
    let q = goal.to_lowercase();
    let topic = if q.contains("app-server") || q.contains("app server") {
        "Codex app-server"
    } else if q.contains("plugin") || q.contains("插件") {
        "Codex plugins"
    } else if q.contains("responses") {
        "Responses API"
    } else {
        "Codex"
    };
    match tool {
        "search_openai_docs" => Ok(json!({"query":topic,"limit":1})),
        "list_openai_docs" => Ok(json!({"limit":1})),
        "list_api_endpoints" => Ok(json!({})),
        _ => Err("MCP_TOOL_ARGUMENT_POLICY_UNSUPPORTED".into()),
    }
}
pub fn endpoint(url: &str) -> Result<()> {
    if matches!(
        url,
        "https://developers.openai.com/mcp" | "https://mcp.expo.dev/mcp"
    ) {
        return Ok(());
    }
    #[cfg(test)]
    if url.starts_with("http://127.0.0.1:") {
        return Ok(());
    }
    Err("PUBLIC_ENDPOINT_REQUIRES_PROVIDER_REVIEW_NO_PRIVATE_OR_ARBITRARY_URL".into())
}
pub fn tool_policy(t: &Value) -> Result<()> {
    if t["annotations"]["readOnlyHint"] != true
        || t["annotations"]["destructiveHint"] == true
        || !matches!(
            t["name"].as_str(),
            Some("search_openai_docs" | "list_openai_docs" | "list_api_endpoints")
        )
    {
        return Err("READ_ONLY_TOOL_CONTRACT_NOT_APPROVED".into());
    }
    Ok(())
}
/// Conservative JSON Schema subset. Unsupported constraints fail closed.
pub fn schema(schema: &Value, value: &Value) -> Result<()> {
    validate_schema(schema, value, 0)
}
fn validate_schema(s: &Value, v: &Value, depth: usize) -> Result<()> {
    if depth > 12 || !s.is_object() {
        return Err("MCP_SCHEMA_UNSUPPORTED".into());
    }
    for k in s.as_object().unwrap().keys() {
        if ![
            "$schema",
            "type",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minLength",
            "maxLength",
            "minimum",
            "maximum",
            "enum",
            "description",
            "title",
            "default",
            "minItems",
            "maxItems",
        ]
        .contains(&k.as_str())
        {
            return Err("MCP_SCHEMA_UNSUPPORTED".into());
        }
    }
    if let Some(en) = s.get("enum") {
        if !en.as_array().is_some_and(|xs| xs.contains(v)) {
            return Err("MCP_SCHEMA_ENUM".into());
        }
    }
    match s["type"].as_str() {
        Some("object") => {
            let o = v.as_object().ok_or("MCP_INPUT_SCHEMA_OBJECT")?;
            let props = s["properties"].as_object().ok_or("MCP_SCHEMA_PROPERTIES")?;
            if let Some(required) = s.get("required") {
                let req = required.as_array().ok_or("MCP_SCHEMA_REQUIRED")?;
                if req.iter().any(|k| {
                    k.as_str()
                        .is_none_or(|k| !props.contains_key(k) || !o.contains_key(k))
                }) {
                    return Err("MCP_INPUT_REQUIRED".into());
                }
            }
            for (k, x) in o {
                let ps = props.get(k).ok_or("MCP_INPUT_UNKNOWN_FIELD")?;
                validate_schema(ps, x, depth + 1)?;
            }
        }
        Some("string") => {
            let text = v.as_str().ok_or("MCP_INPUT_STRING")?;
            if text.len() < s["minLength"].as_u64().unwrap_or(0) as usize
                || text.len() > s["maxLength"].as_u64().unwrap_or(20000) as usize
            {
                return Err("MCP_INPUT_STRING_LIMIT".into());
            }
        }
        Some("integer") | Some("number") => {
            let n = v.as_f64().ok_or("MCP_INPUT_NUMBER")?;
            if s["type"] == "integer" && !v.is_i64() && !v.is_u64() {
                return Err("MCP_INPUT_INTEGER".into());
            }
            if n < s["minimum"].as_f64().unwrap_or(f64::NEG_INFINITY)
                || n > s["maximum"].as_f64().unwrap_or(f64::INFINITY)
            {
                return Err("MCP_INPUT_RANGE".into());
            }
        }
        Some("boolean") => {
            if !v.is_boolean() {
                return Err("MCP_INPUT_BOOLEAN".into());
            }
        }
        Some("array") => {
            let xs = v.as_array().ok_or("MCP_INPUT_ARRAY")?;
            if xs.len() < s["minItems"].as_u64().unwrap_or(0) as usize
                || xs.len() > s["maxItems"].as_u64().unwrap_or(100) as usize
            {
                return Err("MCP_INPUT_ARRAY_LIMIT".into());
            }
            for x in xs {
                validate_schema(&s["items"], x, depth + 1)?
            }
        }
        Some("null") => {
            if !v.is_null() {
                return Err("MCP_INPUT_NULL".into());
            }
        }
        _ => return Err("MCP_SCHEMA_UNSUPPORTED".into()),
    }
    Ok(())
}
pub struct Client {
    client: reqwest::Client,
    url: String,
    session: Option<String>,
    protocol: Option<String>,
    fixture_token: Option<String>,
    id: u64,
    control: Arc<AtomicU8>,
    pub events: Vec<String>,
}
impl Client {
    fn new(url: &str, control: Arc<AtomicU8>) -> Result<Self> {
        endpoint(url)?;
        Self::build(url, control, None)
    }
    pub(in crate::orchestration) fn owned_fixture(
        url: &str,
        token: &str,
        control: Arc<AtomicU8>,
    ) -> Result<Self> {
        let parsed = reqwest::Url::parse(url).map_err(|_| "MCP_FIXTURE_ENDPOINT")?;
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.port().is_none()
            || parsed.path() != "/mcp"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || uuid::Uuid::parse_str(token).is_err()
        {
            return Err("MCP_FIXTURE_ENDPOINT".into());
        }
        Self::build(url, control, Some(token.into()))
    }
    fn build(url: &str, control: Arc<AtomicU8>, fixture_token: Option<String>) -> Result<Self> {
        let timeout = if fixture_token.is_some() { 5 } else { 20 };
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(timeout))
                .build()
                .map_err(|_| "MCP_CLIENT")?,
            url: url.into(),
            session: None,
            protocol: None,
            fixture_token,
            id: 0,
            control,
            events: vec![],
        })
    }
    pub(in crate::orchestration) async fn request(
        &mut self,
        method: &str,
        params: Value,
        notification: bool,
    ) -> Result<Value> {
        if self.control.load(Ordering::SeqCst) != 0 {
            return Err("MCP_CANCELLED".into());
        }
        self.id += 1;
        let id = self.id;
        let mut msg = json!({"jsonrpc":"2.0","method":method,"params":params});
        if !notification {
            msg["id"] = json!(id)
        }
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(&msg);
        if let Some(protocol) = &self.protocol {
            request = request.header("MCP-Protocol-Version", protocol);
        }
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session)
        }
        if let Some(token) = &self.fixture_token {
            request = request.header("X-SkillVerse-Fixture", token);
        }
        let control = self.control.clone();
        let cancelled = async move {
            loop {
                if control.load(Ordering::SeqCst) != 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await
            }
        };
        let mut response = tokio::select! {r=request.send()=>r.map_err(|e|if e.is_timeout(){"MCP_TIMEOUT"}else{"MCP_CONNECTION_FAILED"})?,_ = cancelled=>return Err("MCP_CANCELLED".into())};
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("AUTH_REQUIRED_NOT_VERIFIED".into());
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err("ACCESS_FORBIDDEN_AUTH_OR_PROVIDER_POLICY_NOT_VERIFIED".into());
        }
        if !response.status().is_success() {
            if response.status() == reqwest::StatusCode::NOT_FOUND && self.session.is_some() {
                self.session = None;
                return Err("MCP_SESSION_EXPIRED_NEW_APPROVAL_REQUIRED".into());
            }
            return Err(format!("MCP_HTTP_{}", response.status().as_u16()));
        }
        if let Some(header) = response.headers().get("mcp-session-id") {
            let session = header.to_str().map_err(|_| "MCP_SESSION_INVALID")?;
            if session.is_empty()
                || session.len() > 1024
                || !session.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            {
                return Err("MCP_SESSION_INVALID".into());
            }
            match &self.session {
                None if method == "initialize" => self.session = Some(session.into()),
                Some(current) if current == session => {}
                _ => return Err("MCP_SESSION_CHANGED".into()),
            }
        }
        self.events.push(method.into());
        if notification {
            return Ok(Value::Null);
        }
        let media_type = response
            .headers()
            .get("content-type")
            .and_then(|x| x.to_str().ok())
            .and_then(|x| x.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let sse = media_type == "text/event-stream";
        if !sse && media_type != "application/json" {
            return Err("MCP_RESPONSE_MEDIA_TYPE".into());
        }
        let mut bytes = Vec::new();
        let start = std::time::Instant::now();
        loop {
            if self.control.load(Ordering::SeqCst) != 0 {
                return Err("MCP_CANCELLED".into());
            }
            if start.elapsed() > Duration::from_secs(20) {
                return Err("MCP_TIMEOUT".into());
            }
            let next = tokio::time::timeout(Duration::from_millis(200), response.chunk()).await;
            match next {
                Err(_) => continue,
                Ok(Err(_)) => return Err("MCP_DISCONNECTED".into()),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(chunk))) => {
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() > 262144 {
                        return Err("MCP_RESPONSE_LIMIT".into());
                    }
                    if sse && let Some(reply) = sse_reply(&bytes, id)? {
                        return result(reply, id);
                    }
                }
            }
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| "MCP_INVALID_JSON_OR_SSE")?;
        result(v, id)
    }
    pub(in crate::orchestration) async fn connect(&mut self) -> Result<Value> {
        let init=self.request("initialize",json!({"protocolVersion":MCP_PREFERRED_VERSION,"capabilities":{},"clientInfo":{"name":"skillverse-ai","version":"9.5"}}),false).await?;
        let version = negotiated_mcp_version(&init["protocolVersion"])
            .ok_or("MCP_PROTOCOL_VERSION_UNSUPPORTED")?;
        self.protocol = Some(version.into());
        self.request("notifications/initialized", json!({}), true)
            .await?;
        Ok(init)
    }
    async fn close(&mut self) {
        if let (Some(session), Some(version)) = (self.session.take(), self.protocol.as_deref()) {
            let _ = self
                .client
                .delete(&self.url)
                .header("Mcp-Session-Id", session)
                .header("MCP-Protocol-Version", version)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            self.events.push("session/delete".into());
        }
    }
}
fn sse_reply(bytes: &[u8], id: u64) -> Result<Option<Value>> {
    let Ok(body) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    let normalized = body.replace("\r\n", "\n");
    let mut frames = normalized.split("\n\n").peekable();
    while let Some(frame) = frames.next() {
        if frames.peek().is_none() {
            break;
        }
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|line| line.strip_prefix(' ').unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| "MCP_INVALID_SSE_EVENT")?;
        if value.get("method").is_some() && value.get("id").is_some() {
            return Err("MCP_UNAPPROVED_SERVER_REQUEST".into());
        }
        if value["id"] == id && value.get("method").is_none() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn result(v: Value, id: u64) -> Result<Value> {
    if v["jsonrpc"] != "2.0" || v["id"] != id || v.get("method").is_some() {
        return Err("MCP_RESPONSE_ID_OR_VERSION".into());
    }
    if v.get("error").is_some() {
        return Err("MCP_RPC_ERROR".into());
    }
    v.get("result").cloned().ok_or("MCP_MISSING_RESULT".into())
}
pub async fn discover(url: &str, control: Arc<AtomicU8>) -> Result<Value> {
    let mut c = Client::new(url, control)?;
    let result=async{let init=c.connect().await?;let tools=c.request("tools/list",json!({}),false).await?;if tools["nextCursor"].is_string(){return Err("MCP_PAGINATED_DISCOVERY_REQUIRES_REVIEW".into())}let xs=tools["tools"].as_array().filter(|xs|xs.len()<=100).ok_or("MCP_TOOLS_SCHEMA")?;Ok(json!({"serverInfo":init["serverInfo"],"protocolVersion":init["protocolVersion"],"tools":xs,"events":c.events}))}.await;
    c.close().await;
    result
}
pub async fn execute(
    p: &Profile,
    goal: &str,
    control: Arc<AtomicU8>,
    progress: tokio::sync::mpsc::UnboundedSender<&'static str>,
) -> Result<Value> {
    let _ = progress.send("NODE_CONNECTING");
    let mut c = Client::new(&p.endpoint, control)?;
    let outcome=async{
 let init=c.connect().await?;let tools=c.request("tools/list",json!({}),false).await?;let t=tools["tools"].as_array().and_then(|xs|xs.iter().find(|t|t["name"]==p.tool["name"])).ok_or("MCP_TOOL_NO_LONGER_AVAILABLE")?;tool_policy(t)?;if t["inputSchema"]!=p.tool["inputSchema"]||t.get("outputSchema")!=p.tool.get("outputSchema"){return Err("MCP_TOOL_SCHEMA_CHANGED_REVIEW_REQUIRED".into())}
 let _=progress.send("NODE_CALLABLE");
 let arguments=public_arguments(t["name"].as_str().ok_or("MCP_TOOL_NAME")?,goal)?;schema(&t["inputSchema"],&arguments)?;
 let _=progress.send("NODE_TOOL_RUNNING");
 let output=c.request("tools/call",json!({"name":t["name"],"arguments":arguments}),false).await?;
 if output["isError"]==true{return Err("MCP_TOOL_REPORTED_ERROR".into())}let content=output["content"].as_array().filter(|x|!x.is_empty()&&x.len()<=32).ok_or("MCP_OUTPUT_CONTENT_SCHEMA")?;
 if content.iter().any(|x|x["type"]!="text"||!x["text"].as_str().is_some_and(|s|!s.is_empty())){return Err("MCP_OUTPUT_CONTENT_SCHEMA".into())}
 let has_schema=if let Some(s)=t.get("outputSchema"){schema(s,&output["structuredContent"])?;true}else{false};
 Ok(json!({"scope":"REVIEWED_PUBLIC_READ_ONLY_MCP_TOOL_CALL","provider":p.endpoint,"serverInfo":init["serverInfo"],"protocolVersion":init["protocolVersion"],"transport":"streamable-http","states":{"CONFIGURED":true,"CONNECTED":true,"TOOLS_DISCOVERED":true,"CALLABLE":true,"EXECUTED":true,"VERIFIED":true},"tool":t["name"],"inputSchemaVerified":true,"inputDigest":digest(arguments.to_string().as_bytes()),"outputSchema":if has_schema{"DECLARED_SCHEMA_VALIDATED"}else{"NOT_PROVIDED_BY_SERVER_CONTENT_ENVELOPE_ONLY"},"result":sanitized(&output),"events":c.events,"permission":"PUBLIC_ALLOWLISTED_READ_ONLY_NO_ACCOUNT","resultVerification":"PROTOCOL_ID_SCHEMA_AND_NONEMPTY_TEXT_NOT_GENERAL_FACT_VERIFICATION","externalInstructionsTrusted":false,"modelCalls":0}))
 }.await;
    c.close().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Mutex, atomic::AtomicBool},
    };
    struct Peer {
        url: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        methods: Arc<Mutex<Vec<String>>>,
    }
    impl Peer {
        fn new(mode: u8) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            let methods = Arc::new(Mutex::new(Vec::new()));
            let tracked = methods.clone();
            let thread = std::thread::spawn(move || {
                while !stop2.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_millis(300)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut buf = [0u8; 2048];
                    let mut target = usize::MAX;
                    while bytes.len() < target {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => bytes.extend_from_slice(&buf[..n]),
                        }
                        if let Some(i) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                            let header = String::from_utf8_lossy(&bytes[..i]);
                            let len = header
                                .lines()
                                .find_map(|l| {
                                    l.to_lowercase()
                                        .strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            target = i + 4 + len;
                        }
                        if bytes.len() > 131072 {
                            break;
                        }
                    }
                    let Some(i) = bytes.windows(4).position(|x| x == b"\r\n\r\n") else {
                        continue;
                    };
                    let header = String::from_utf8_lossy(&bytes[..i]);
                    if header.starts_with("DELETE") {
                        tracked.lock().unwrap().push("DELETE".into());
                        let _ = stream
                            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    let Ok(v) = serde_json::from_slice::<Value>(&bytes[i + 4..]) else {
                        continue;
                    };
                    let method = v["method"].as_str().unwrap_or("");
                    tracked.lock().unwrap().push(method.into());
                    if method != "initialize"
                        && !header
                            .to_ascii_lowercase()
                            .contains("mcp-protocol-version: 2025-06-18")
                    {
                        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    if mode == 7 && method == "tools/call" {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    if mode == 2 {
                        let _=stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    if mode == 3 {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    if mode == 4 {
                        continue;
                    }
                    if v.get("id").is_none() {
                        let _=stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    let result = match method {
                        "initialize" => {
                            json!({"protocolVersion":if mode==5{"unreviewed-version"}else{"2025-06-18"},"capabilities":{"tools":{}},"serverInfo":{"name":"explicit-fault-fixture","version":"1"}})
                        }
                        "tools/list" => json!({"tools":[tool()]}),
                        "tools/call" => {
                            if mode == 6 {
                                json!({"isError":true,"content":[{"type":"text","text":"Forged success must not override isError"}]})
                            } else {
                                json!({"content":[{"type":"text","text":"Synthetic public documentation result"}],"isError":false})
                            }
                        }
                        _ => json!({}),
                    };
                    let message = json!({"jsonrpc":"2.0","id":v["id"],"result":result}).to_string();
                    let body = if mode == 1 {
                        format!("event: message\r\ndata: {message}\r\n\r\n")
                    } else {
                        message
                    };
                    let content_type = if mode == 1 {
                        "text/event-stream"
                    } else {
                        "application/json"
                    };
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(headers.as_bytes());
                    let split = body.len() / 2;
                    let _ = stream.write_all(&body.as_bytes()[..split]);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(3));
                    let _ = stream.write_all(&body.as_bytes()[split..]);
                }
            });
            Self {
                url,
                stop,
                thread: Some(thread),
                methods,
            }
        }
    }
    impl Drop for Peer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                t.join().unwrap()
            }
        }
    }
    fn tool() -> Value {
        json!({"name":"search_openai_docs","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]},"annotations":{"readOnlyHint":true,"destructiveHint":false}})
    }
    fn profile(url: &str) -> Profile {
        Profile {
            id: "test".into(),
            kind: "mcp".into(),
            name: "explicit fixture".into(),
            root: String::new(),
            component: String::new(),
            repository: "test-only".into(),
            commit: "test-only".into(),
            enabled: true,
            mode: "PUBLIC_READ_ONLY_MCP".into(),
            files: BTreeMap::new(),
            tool: tool(),
            endpoint: url.into(),
            plugin: Value::Null,
            reviewed_at: 0,
            blockers: vec![],
        }
    }
    #[tokio::test]
    async fn json_and_fragmented_sse_handshake_discovery_call_and_session_release() {
        for mode in [0, 1] {
            let peer = Peer::new(mode);
            let (tx, _) = tokio::sync::mpsc::unbounded_channel();
            let out = execute(
                &profile(&peer.url),
                "synthetic query",
                Arc::new(AtomicU8::new(0)),
                tx,
            )
            .await
            .unwrap();
            assert_eq!(out["states"]["EXECUTED"], true);
            assert_eq!(out["protocolVersion"], "2025-06-18");
            let methods = peer.methods.lock().unwrap();
            assert!(methods.contains(&"tools/call".into()));
            assert_eq!(methods.last().unwrap(), "DELETE");
        }
    }
    #[tokio::test]
    async fn expired_session_stops_without_reconnecting_or_replaying_the_tool() {
        let peer = Peer::new(7);
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            execute(&profile(&peer.url), "query", Arc::new(AtomicU8::new(0)), tx)
                .await
                .unwrap_err(),
            "MCP_SESSION_EXPIRED_NEW_APPROVAL_REQUIRED"
        );
        assert_eq!(
            &*peer.methods.lock().unwrap(),
            &[
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call"
            ]
        );
        let fresh = Peer::new(0);
        assert_eq!(
            discover(&fresh.url, Arc::new(AtomicU8::new(0)))
                .await
                .unwrap()["protocolVersion"],
            "2025-06-18"
        );
        assert!(
            !fresh
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|method| method == "tools/call")
        );
    }
    #[test]
    fn sse_requires_a_complete_event_and_blocks_server_requests() {
        assert!(
            sse_reply(b"data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}", 3)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            sse_reply(
                b"data: {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"sampling/createMessage\"}\n\n",
                3
            )
            .unwrap_err(),
            "MCP_UNAPPROVED_SERVER_REQUEST"
        );
    }
    #[tokio::test]
    async fn authentication_and_unsupported_protocol_never_reach_tool_call() {
        for (mode, error) in [
            (2, "AUTH_REQUIRED_NOT_VERIFIED"),
            (5, "MCP_PROTOCOL_VERSION_UNSUPPORTED"),
        ] {
            let peer = Peer::new(mode);
            assert_eq!(
                discover(&peer.url, Arc::new(AtomicU8::new(0)))
                    .await
                    .unwrap_err(),
                error
            );
            assert!(!peer.methods.lock().unwrap().contains(&"tools/call".into()));
        }
    }
    #[tokio::test]
    async fn connection_failure_timeout_cancel_and_disconnect_are_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(
            discover(&url, Arc::new(AtomicU8::new(0)))
                .await
                .unwrap_err(),
            "MCP_CONNECTION_FAILED"
        );
        let peer = Peer::new(3);
        let mut c = Client::new(&peer.url, Arc::new(AtomicU8::new(0))).unwrap();
        c.client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        assert_eq!(c.connect().await.unwrap_err(), "MCP_TIMEOUT");
        let peer = Peer::new(3);
        let control = Arc::new(AtomicU8::new(0));
        let copy = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            copy.store(2, Ordering::SeqCst)
        });
        assert_eq!(
            discover(&peer.url, control).await.unwrap_err(),
            "MCP_CANCELLED"
        );
        let peer = Peer::new(4);
        assert!(
            discover(&peer.url, Arc::new(AtomicU8::new(0)))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn forged_tool_success_text_cannot_override_error_or_changed_schema() {
        let peer = Peer::new(6);
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            execute(&profile(&peer.url), "query", Arc::new(AtomicU8::new(0)), tx)
                .await
                .unwrap_err(),
            "MCP_TOOL_REPORTED_ERROR"
        );
        let peer = Peer::new(0);
        let mut p = profile(&peer.url);
        p.tool["inputSchema"]["properties"]["query"]["minLength"] = json!(1);
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            execute(&p, "query", Arc::new(AtomicU8::new(0)), tx)
                .await
                .unwrap_err(),
            "MCP_TOOL_SCHEMA_CHANGED_REVIEW_REQUIRED"
        );
        assert!(!peer.methods.lock().unwrap().contains(&"tools/call".into()));
    }
}
