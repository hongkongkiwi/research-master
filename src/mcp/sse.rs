//! Legacy HTTP + SSE transport for the MCP server (spec revision 2024-11-05).
//!
//! Streamable HTTP (see `server.rs`) is the current MCP transport, but many
//! clients still speak only the original HTTP transport: one long `GET /sse`
//! connection carries JSON-RPC responses as Server-Sent Events, while requests
//! travel as `POST /messages?sessionId=...` and return 202 Accepted.
//!
//! The HTTP layer is hand-rolled on tokio, mirroring `panel.rs`, so no HTTP
//! framework dependency is added. Each SSE connection gets a fresh pmcp
//! `Server` (clean initialization state per client) driven over in-memory
//! channels through pmcp's public [`Transport`] trait, using the same
//! JSON-RPC wire format as the other transports.

use crate::mcp::server::McpServer;
use crate::mcp::tools::ToolRegistry;
use pmcp::shared::Transport;
use pmcp::types::TransportMessage;
use pmcp::{Error, StdioTransport};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const MAX_HEAD_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_EVERY: Duration = Duration::from_secs(30);

/// Outbound queue for one SSE session: pmcp Server -> client stream.
type OutboundTx = mpsc::UnboundedSender<TransportMessage>;
type Sessions = Arc<Mutex<HashMap<String, OutboundTx>>>;

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_session_id() -> String {
    let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032x}{n:04x}")
}

/// Bind the legacy SSE transport and serve it until the returned handle is
/// aborted. Bearer auth follows the streamable HTTP transport: when
/// `RESEARCH_MASTER_HTTP_TOKEN` is set, every request must present it.
pub async fn run(tools: ToolRegistry, addr: &str) -> Result<(SocketAddr, JoinHandle<()>), Error> {
    let auth_token = std::env::var("RESEARCH_MASTER_HTTP_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty());
    run_with_auth(tools, addr, auth_token).await
}

/// Like [`run`], with the bearer token supplied directly (test seam).
pub async fn run_with_auth(
    tools: ToolRegistry,
    addr: &str,
    auth_token: Option<String>,
) -> Result<(SocketAddr, JoinHandle<()>), Error> {
    let socket_addr: SocketAddr = addr
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid address: {}", e)))?;
    let listener = TcpListener::bind(socket_addr)
        .await
        .map_err(|e| Error::internal(format!("Cannot bind {socket_addr}: {e}")))?;
    let local = listener
        .local_addr()
        .map_err(|e| Error::internal(e.to_string()))?;
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(connected) => connected,
                Err(err) => {
                    tracing::warn!("SSE transport accept failed: {err}");
                    continue;
                }
            };
            let sessions = Arc::clone(&sessions);
            let tools = tools.clone();
            let auth_token = auth_token.clone();
            tokio::spawn(handle_connection(stream, tools, sessions, auth_token));
        }
    });
    Ok((local, handle))
}

/// In-memory transport connecting one SSE session's POST /messages feed to a
/// dedicated pmcp Server task.
#[derive(Debug)]
struct ChannelTransport {
    inbound: mpsc::UnboundedReceiver<TransportMessage>,
    outbound: OutboundTx,
}

#[async_trait::async_trait]
impl Transport for ChannelTransport {
    async fn send(&mut self, message: TransportMessage) -> Result<(), Error> {
        self.outbound
            .send(message)
            .map_err(|_| Error::internal("SSE session closed"))
    }

    async fn receive(&mut self) -> Result<TransportMessage, Error> {
        self.inbound
            .recv()
            .await
            .ok_or_else(|| Error::internal("SSE session closed"))
    }

    async fn close(&mut self) -> Result<(), Error> {
        self.inbound.close();
        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "sse"
    }
}

#[derive(Debug)]
enum HeadRead {
    /// Head text plus any body bytes already read from the socket.
    Complete(String, Vec<u8>),
    TooLarge,
    Empty,
    Partial,
}

// Reads directly from the stream (no buffering) and carries any bytes past the
// head back to the caller so a body arriving in the same TCP segment as the
// head is not lost.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<HeadRead> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find_head_end(&buf) {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            return Ok(HeadRead::Complete(head, buf[end + 4..].to_vec()));
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return Ok(HeadRead::TooLarge);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return if buf.is_empty() {
                Ok(HeadRead::Empty)
            } else {
                Ok(HeadRead::Partial)
            };
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Request target split into path and raw query string.
fn split_target(target: &str) -> (String, Option<String>) {
    match target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (target.to_string(), None),
    }
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

fn authorized(head: &str, auth_token: &Option<String>) -> bool {
    match auth_token {
        None => true,
        Some(expected) => header_value(head, "authorization").is_some_and(|value| {
            value == format!("Bearer {expected}") || value == format!("bearer {expected}")
        }),
    }
}

fn content_length(head: &str) -> Option<usize> {
    header_value(head, "content-length").and_then(|v| v.parse().ok())
}

async fn read_body(
    stream: &mut TcpStream,
    head: &str,
    mut leftover: Vec<u8>,
) -> Result<Vec<u8>, BodyError> {
    let len = content_length(head).ok_or(BodyError::MissingLength)?;
    if len > MAX_BODY_BYTES {
        return Err(BodyError::TooLarge);
    }
    if leftover.len() > len {
        leftover.truncate(len);
    }
    if leftover.len() < len {
        let mut rest = vec![0u8; len - leftover.len()];
        stream
            .read_exact(&mut rest)
            .await
            .map_err(|_| BodyError::Truncated)?;
        leftover.extend_from_slice(&rest);
    }
    Ok(leftover)
}

enum BodyError {
    MissingLength,
    TooLarge,
    Truncated,
}

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn jsonrpc_error_body(code: i32, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message},
        "id": null,
    })
    .to_string()
}

async fn send_jsonrpc_error(stream: &mut TcpStream, status: &str, code: i32, message: &str) {
    let response = http_response(
        status,
        "application/json",
        &jsonrpc_error_body(code, message),
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

fn sse_frame(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

async fn handle_connection(
    mut stream: TcpStream,
    tools: ToolRegistry,
    sessions: Sessions,
    auth_token: Option<String>,
) {
    let (head, leftover) = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await {
        Ok(Ok(HeadRead::Complete(head, leftover))) => (head, leftover),
        Ok(Ok(HeadRead::Empty)) => return,
        Ok(Ok(HeadRead::Partial)) => {
            send_jsonrpc_error(&mut stream, "400 Bad Request", -32600, "Incomplete request").await;
            return;
        }
        Ok(Ok(HeadRead::TooLarge)) => {
            send_jsonrpc_error(
                &mut stream,
                "413 Content Too Large",
                -32600,
                "Request head too large",
            )
            .await;
            return;
        }
        Ok(Err(_)) => return,
        Err(_) => {
            send_jsonrpc_error(
                &mut stream,
                "408 Request Timeout",
                -32600,
                "Request timed out",
            )
            .await;
            return;
        }
    };

    let request_line = head.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = split_target(&target);

    if !authorized(&head, &auth_token) {
        send_jsonrpc_error(
            &mut stream,
            "401 Unauthorized",
            -32001,
            "missing or invalid bearer token (set Authorization: Bearer <RESEARCH_MASTER_HTTP_TOKEN>)",
        )
        .await;
        return;
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/sse") => handle_sse_session(stream, tools, sessions).await,
        ("POST", "/messages") => {
            handle_message_post(stream, &head, leftover, query.as_deref(), sessions).await
        }
        ("OPTIONS", _) => {
            let response = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\n\
                 Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                 Access-Control-Allow-Headers: Authorization, Content-Type\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
        _ => {
            send_jsonrpc_error(&mut stream, "404 Not Found", -32600, "Unknown endpoint").await;
        }
    }
}

/// Serve one session: SSE stream out, POST /messages in.
async fn handle_sse_session(mut stream: TcpStream, tools: ToolRegistry, sessions: Sessions) {
    let session_id = new_session_id();
    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();

    // A fresh Server per connection keeps initialize state session-scoped.
    let server = match McpServer::build_server_impl(tools) {
        Ok(server) => server,
        Err(e) => {
            send_jsonrpc_error(
                &mut stream,
                "500 Internal Server Error",
                -32603,
                &format!("Cannot initialize MCP server: {e}"),
            )
            .await;
            return;
        }
    };
    let server_task = tokio::spawn(async move {
        let transport = ChannelTransport {
            inbound: inbound_rx,
            outbound: outbound_tx,
        };
        // The pmcp run loop only ends by being aborted; channel closure stops
        // its message handler.
        let _ = server.run(transport).await;
    });

    if sessions
        .lock()
        .expect("session mutex not poisoned")
        .insert(session_id.clone(), inbound_tx)
        .is_some()
    {
        // Collision of a nanosecond-plus-counter id is not reachable in
        // practice, but never overwrite a live session if it were.
        server_task.abort();
        send_jsonrpc_error(
            &mut stream,
            "500 Internal Server Error",
            -32603,
            "Session id collision",
        )
        .await;
        return;
    }

    // SSE response head; the connection stays open for the session lifetime.
    let sse_headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    if stream.write_all(sse_headers.as_bytes()).await.is_err() {
        sessions
            .lock()
            .expect("session mutex not poisoned")
            .remove(&session_id);
        server_task.abort();
        return;
    }
    let _ = stream.flush().await;

    let endpoint_event = sse_frame("endpoint", &format!("/messages?sessionId={session_id}"));
    if stream.write_all(endpoint_event.as_bytes()).await.is_err() {
        sessions
            .lock()
            .expect("session mutex not poisoned")
            .remove(&session_id);
        server_task.abort();
        return;
    }
    let _ = stream.flush().await;

    loop {
        let message = tokio::select! {
            msg = outbound_rx.recv() => msg,
            _ = tokio::time::sleep(KEEPALIVE_EVERY) => {
                if stream.write_all(b": keepalive\n\n").await.is_err() {
                    break;
                }
                let _ = stream.flush().await;
                continue;
            }
        };
        let Some(message) = message else {
            break;
        };
        let data = match StdioTransport::serialize_message(&message) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to serialize SSE message: {e}");
                break;
            }
        };
        let frame = sse_frame("message", &String::from_utf8_lossy(&data));
        if stream.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        let _ = stream.flush().await;
    }

    // Dropping the inbound sender makes the pmcp session task's receive fail,
    // ending its message handler; abort also stops the keep-alive main loop.
    sessions
        .lock()
        .expect("session mutex not poisoned")
        .remove(&session_id);
    server_task.abort();
}

async fn handle_message_post(
    mut stream: TcpStream,
    head: &str,
    leftover: Vec<u8>,
    query: Option<&str>,
    sessions: Sessions,
) {
    let session_id = query.and_then(|q| query_param(q, "sessionId"));
    let session_id = match session_id.filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => {
            send_jsonrpc_error(
                &mut stream,
                "400 Bad Request",
                -32600,
                "Missing sessionId query parameter",
            )
            .await;
            return;
        }
    };

    let sender = sessions
        .lock()
        .expect("session mutex not poisoned")
        .get(&session_id)
        .cloned();
    let Some(sender) = sender else {
        send_jsonrpc_error(&mut stream, "404 Not Found", -32600, "Unknown session").await;
        return;
    };

    let body = match read_body(&mut stream, head, leftover).await {
        Ok(body) => body,
        Err(BodyError::MissingLength) => {
            send_jsonrpc_error(
                &mut stream,
                "411 Length Required",
                -32700,
                "Content-Length header required",
            )
            .await;
            return;
        }
        Err(BodyError::TooLarge) => {
            send_jsonrpc_error(
                &mut stream,
                "413 Content Too Large",
                -32700,
                "Request body too large",
            )
            .await;
            return;
        }
        Err(BodyError::Truncated) => {
            send_jsonrpc_error(
                &mut stream,
                "400 Bad Request",
                -32700,
                "Request body truncated",
            )
            .await;
            return;
        }
    };

    let message = match StdioTransport::parse_message(&body) {
        Ok(message) => message,
        Err(e) => {
            send_jsonrpc_error(
                &mut stream,
                "400 Bad Request",
                -32700,
                &format!("Invalid JSON-RPC message: {e}"),
            )
            .await;
            return;
        }
    };

    if sender.send(message).is_err() {
        send_jsonrpc_error(&mut stream, "404 Not Found", -32600, "Unknown session").await;
        return;
    }

    let _ = stream
        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{MockSource, SourceRegistry};
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    fn test_registry() -> ToolRegistry {
        let mut registry = SourceRegistry::empty_for_tests();
        registry.register(Arc::new(MockSource::new()));
        ToolRegistry::from_sources(&registry)
    }

    async fn read_until(stream: &mut TcpStream, needle: &str) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            if find_subsequence(&buf, needle.as_bytes()).is_some() {
                // Return everything read so far; assertions may look past the
                // needle into the rest of the frame.
                return String::from_utf8_lossy(&buf).into_owned();
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!(
                    "timed out waiting for {needle:?}, got: {}",
                    String::from_utf8_lossy(&buf)
                );
            }
            let n = tokio::time::timeout(remaining, stream.read(&mut chunk))
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
                .expect("read failed");
            if n == 0 {
                panic!(
                    "stream closed while waiting for {needle:?}, got: {}",
                    String::from_utf8_lossy(&buf)
                );
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    #[tokio::test]
    async fn channel_transport_roundtrips_messages() {
        let (server_to_client_tx, mut server_to_client_rx) = mpsc::unbounded_channel();
        let (client_to_server_tx, client_to_server_rx) = mpsc::unbounded_channel();
        let mut transport = ChannelTransport {
            inbound: client_to_server_rx,
            outbound: server_to_client_tx,
        };

        let request = StdioTransport::parse_message(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
            .expect("parse request");
        client_to_server_tx.send(request.clone()).expect("send in");

        let received = transport.receive().await.expect("receive");
        transport.send(received.clone()).await.expect("send out");
        let forwarded = server_to_client_rx.recv().await.expect("recv out");

        let serialized = StdioTransport::serialize_message(&forwarded).expect("serialize");
        let reparsed: TransportMessage =
            StdioTransport::parse_message(&serialized).expect("reparse");
        match (request, reparsed) {
            (
                TransportMessage::Request { id: id_a, .. },
                TransportMessage::Request { id: id_b, .. },
            ) => assert_eq!(id_a, id_b),
            _ => panic!("expected request messages"),
        }
    }

    #[tokio::test]
    async fn sse_session_completes_initialize_and_tool_call() {
        let (addr, _handle) = run_with_auth(test_registry(), "127.0.0.1:0", None)
            .await
            .expect("bind sse transport");

        // Open the SSE channel and read the endpoint event.
        let mut sse = TcpStream::connect(addr).await.expect("connect");
        sse.write_all(b"GET /sse HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n")
            .await
            .expect("write get");
        let endpoint = read_until(&mut sse, "\n\n").await;
        assert!(
            endpoint.contains("event: endpoint") && endpoint.contains("200 OK"),
            "expected SSE head plus endpoint event, got: {endpoint}"
        );
        let session_id = endpoint
            .lines()
            .find_map(|line| line.strip_prefix("data: /messages?sessionId="))
            .expect("endpoint event carries session id")
            .to_string();
        assert!(!session_id.is_empty());

        // initialize -> response comes back on the SSE channel.
        post_message(
            addr,
            &session_id,
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.0.0"}}}"#,
        )
        .await;
        let init_response = read_until(&mut sse, "\"id\":1").await;
        assert!(
            init_response.contains("research-master"),
            "initialize result should name the server: {init_response}"
        );

        // tools/list over the same session.
        post_message(
            addr,
            &session_id,
            br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .await;
        let tools_response = read_until(&mut sse, "\"id\":2").await;
        assert!(
            tools_response.contains("\"tools\""),
            "tools/list should return tools: {tools_response}"
        );
    }

    #[tokio::test]
    async fn unknown_session_is_rejected_with_404() {
        let (addr, _handle) = run_with_auth(test_registry(), "127.0.0.1:0", None)
            .await
            .expect("bind sse transport");

        let mut conn = TcpStream::connect(addr).await.expect("connect");
        conn.write_all(
            b"POST /messages?sessionId=deadbeef HTTP/1.1\r\nHost: localhost\r\nContent-Length: 36\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}",
        )
        .await
        .expect("write post");
        let response = read_until(&mut conn, "\r\n\r\n").await;
        assert!(response.contains("404"), "expected 404, got: {response}");
    }

    #[tokio::test]
    async fn bearer_token_is_enforced_when_configured() {
        let (addr, _handle) = run_with_auth(test_registry(), "127.0.0.1:0", Some("secret".into()))
            .await
            .expect("bind sse transport");

        // No token: rejected before any session exists.
        let mut conn = TcpStream::connect(addr).await.expect("connect");
        conn.write_all(b"GET /sse HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write get");
        let response = read_until(&mut conn, "\r\n\r\n").await;
        assert!(response.contains("401"), "expected 401, got: {response}");

        // Wrong token: rejected.
        let mut conn = TcpStream::connect(addr).await.expect("connect");
        conn.write_all(
            b"GET /sse HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer wrong\r\n\r\n",
        )
        .await
        .expect("write get");
        let response = read_until(&mut conn, "\r\n\r\n").await;
        assert!(response.contains("401"), "expected 401, got: {response}");

        // Correct token: endpoint event arrives.
        let mut sse = TcpStream::connect(addr).await.expect("connect");
        sse.write_all(
            b"GET /sse HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer secret\r\n\r\n",
        )
        .await
        .expect("write get");
        let endpoint = read_until(&mut sse, "\n\n").await;
        assert!(
            endpoint.contains("event: endpoint"),
            "expected endpoint event with valid token, got: {endpoint}"
        );
    }

    async fn post_message(addr: SocketAddr, session_id: &str, body: &[u8]) {
        let mut conn = TcpStream::connect(addr).await.expect("connect");
        let request = format!(
            "POST /messages?sessionId={session_id} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let mut request = request.into_bytes();
        request.extend_from_slice(body);
        conn.write_all(&request).await.expect("write post");
        let response = read_until(&mut conn, "\r\n\r\n").await;
        assert!(
            response.contains("202"),
            "POST /messages should return 202, got: {response}"
        );
    }
}
