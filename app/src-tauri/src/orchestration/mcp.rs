//! First-party loopback MCP conformance exercise. Never connects to user MCPs.
//! The trusted tool contract is compiled here; server annotations alone grant no permission.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
const VERSION: &str = "2025-03-26";
fn tool() -> Value {
    json!({"name":"sum_fixture","description":"Sum three public fixture values; no filesystem, account or network side effects","inputSchema":{"type":"object","properties":{"values":{"type":"array","items":{"type":"integer","minimum":0,"maximum":1000},"minItems":3,"maxItems":3}},"required":["values"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true}})
}
fn arguments(value: &Value) -> Result<Vec<i64>> {
    let map = value.as_object().ok_or("MCP_ARGUMENT_SCHEMA")?;
    if map.len() != 1 {
        return Err("MCP_ARGUMENT_SCHEMA".into());
    }
    let values = map
        .get("values")
        .and_then(Value::as_array)
        .ok_or("MCP_ARGUMENT_SCHEMA")?;
    if values.len() != 3 {
        return Err("MCP_ARGUMENT_SCHEMA".into());
    }
    values
        .iter()
        .map(|v| {
            v.as_i64()
                .filter(|n| (0..=1000).contains(n))
                .ok_or("MCP_ARGUMENT_SCHEMA".into())
        })
        .collect()
}
struct Fixture {
    endpoint: String,
    nonce: String,
    stop: Arc<AtomicBool>,
    trace: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn start(fault: u8) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "MCP_FIXTURE_BIND")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "MCP_FIXTURE_BIND")?;
        let endpoint = format!(
            "http://{}/mcp",
            listener.local_addr().map_err(|_| "MCP_FIXTURE_BIND")?
        );
        let nonce = uuid::Uuid::new_v4().to_string();
        let token = nonce.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let trace_writer = trace.clone();
        let thread = std::thread::spawn(move || {
            let mut initialized = false;
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Windows accepted sockets can retain the listener's
                        // nonblocking mode. Body fragments must wait for data,
                        // not be treated as a disconnected MCP transport.
                        if stream.set_nonblocking(false).is_err() {
                            trace_writer
                                .lock()
                                .unwrap()
                                .push("socket-mode-error".into());
                            continue;
                        }
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let request = read_request(&mut stream);
                        let Ok((header, body)) = request else {
                            trace_writer
                                .lock()
                                .unwrap()
                                .push(format!("read-error:{}", request.unwrap_err()));
                            continue;
                        };
                        if !header.contains(&format!("x-skillverse-fixture: {token}")) {
                            trace_writer.lock().unwrap().push("scope-rejected".into());
                            continue;
                        }
                        let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
                            continue;
                        };
                        let method = msg["method"].as_str().unwrap_or("");
                        trace_writer.lock().unwrap().push(method.into());
                        if fault == 1 && method == "tools/call" {
                            break;
                        }
                        let result = match method {
                            "initialize" => {
                                json!({"protocolVersion":VERSION,"serverInfo":{"name":"skillverse-owned-fixture","version":"1"},"capabilities":{"tools":{}}})
                            }
                            "notifications/initialized" => {
                                initialized = true;
                                Value::Null
                            }
                            "tools/list" if initialized => json!({"tools":[tool()]}),
                            "tools/call"
                                if initialized && msg["params"]["name"] == "sum_fixture" =>
                            {
                                match arguments(&msg["params"]["arguments"]) {
                                    Ok(values) => {
                                        json!({"content":[{"type":"text","text":json!({"total":if fault==2{999}else{values.iter().sum::<i64>()},"rowCount":values.len()}).to_string()}],"isError":false})
                                    }
                                    Err(_) => {
                                        json!({"isError":true,"content":[{"type":"text","text":"Invalid arguments"}]})
                                    }
                                }
                            }
                            _ => json!({"isError":true}),
                        };
                        let (status, body) = if method == "notifications/initialized" {
                            ("202 Accepted", String::new())
                        } else {
                            (
                                "200 OK",
                                json!({"jsonrpc":"2.0","id":msg["id"],"result":result}).to_string(),
                            )
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let outcome = stream.write_all(response.as_bytes());
                        trace_writer.lock().unwrap().push(if let Err(e) = outcome {
                            format!("write-error:{:?}", e.kind())
                        } else {
                            "response-written".into()
                        });
                        // Finish the HTTP response before closing the connection.
                        // A bare socket drop can turn a correctly rejected forged
                        // result into a transport reset on Windows under load.
                        let _ = stream.flush();
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                        let mut drain = [0u8; 512];
                        while let Ok(n) = stream.read(&mut drain) {
                            if n == 0 {
                                break;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            endpoint,
            nonce,
            stop,
            trace,
            thread: Some(thread),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    let (header_end, length) = loop {
        let n = stream.read(&mut buffer).map_err(|_| "MCP_REQUEST_READ")?;
        if n == 0 || bytes.len() + n > 32768 {
            return Err("MCP_REQUEST_LIMIT".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..i]).to_lowercase();
            let length = header
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .ok_or("MCP_HTTP_LENGTH")?;
            if length > 16384 {
                return Err("MCP_REQUEST_LIMIT".into());
            }
            break (i + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let n = stream.read(&mut buffer).map_err(|_| "MCP_REQUEST_READ")?;
        if n == 0 {
            return Err("MCP_DISCONNECTED".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
        if bytes.len() > 32768 {
            return Err("MCP_REQUEST_LIMIT".into());
        }
    }
    Ok((
        String::from_utf8_lossy(&bytes[..header_end]).to_lowercase(),
        bytes[header_end..header_end + length].to_vec(),
    ))
}
pub(super) async fn exercise(control: Arc<AtomicU8>, fault: u8) -> Result<Value> {
    let fixture = Fixture::start(fault)?;
    use super::compatibility::remote::{Client, schema};
    let mut client = Client::owned_fixture(&fixture.endpoint, &fixture.nonce, control)?;
    let init = client.connect().await?;
    if init["serverInfo"]["name"] != "skillverse-owned-fixture" {
        return Err("MCP_NEGOTIATION_FAILED".into());
    }
    let tools = client.request("tools/list", json!({}), false).await?;
    if tools["tools"] != json!([tool()]) {
        return Err("MCP_UNREVIEWED_TOOL_SCHEMA".into());
    }
    let args = json!({"values":[13,21,34]});
    schema(&tool()["inputSchema"], &args)?;
    let values = arguments(&args)?;
    let result = client
        .request(
            "tools/call",
            json!({"name":"sum_fixture","arguments":args}),
            false,
        )
        .await?;
    let output: Value = serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .ok_or("MCP_RESULT_SCHEMA")?,
    )
    .map_err(|_| "MCP_RESULT_SCHEMA")?;
    if result["isError"] != false
        || output["total"] != values.iter().sum::<i64>()
        || output["rowCount"] != 3
    {
        return Err("MCP_RESULT_VERIFICATION_FAILED".into());
    }
    Ok(
        json!({"scope":"FIRST_PARTY_OWNED_LOOPBACK_MCP_ONLY_NOT_USER_MCP_ACCEPTANCE","protocolVersion":init["protocolVersion"],"transport":"streamable-http","protocolEvents":client.events,"fixtureTrace":*fixture.trace.lock().unwrap(),"connected":true,"toolsDiscovered":1,"schemaVerified":true,"permission":"COMPILED_READ_ONLY_TOOL_CONTRACT_AND_APPROVED_WORKFLOW","toolCalled":"sum_fixture","resultVerified":true,"total":output["total"],"toolSchemaDigest":digest(tool().to_string().as_bytes()),"outputDigest":digest(result.to_string().as_bytes()),"userMcpRuntime":"NOT VERIFIED","network":"OWNED_LOOPBACK_ONLY","account":false}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_loopback_mcp_negotiation_discovery_schema_call_disconnect_and_forged_result() {
        let c = Arc::new(AtomicU8::new(0));
        assert_eq!(exercise(c.clone(), 0).await.unwrap()["total"], 68);
        assert!(
            exercise(c.clone(), 1)
                .await
                .unwrap_err()
                .contains("MCP_CONNECTION_FAILED")
        );
        for _ in 0..5 {
            assert_eq!(
                exercise(c.clone(), 2).await.unwrap_err(),
                "MCP_RESULT_VERIFICATION_FAILED"
            );
        }
        assert!(arguments(&json!({"values":[1,2,3],"command":"unknown"})).is_err());
    }
}
