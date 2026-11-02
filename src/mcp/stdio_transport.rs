//! Spec-compliant newline-delimited stdio transport for MCP.
//!
//! The MCP stdio transport (modelcontextprotocol.io) delimits JSON-RPC
//! messages with newlines. pmcp 1.9.4's built-in `StdioTransport` instead
//! speaks LSP-style `Content-Length` framing, which standard MCP clients do
//! not send, so stdio mode never interoperates. This module implements the
//! trait with the correct wire format and reuses pmcp's message
//! serialization/parsing so all message types are handled identically.

use pmcp::error::TransportError;
use pmcp::shared::transport::TransportMessage;
use pmcp::{Result, Transport};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Newline-delimited JSON-RPC transport over stdin/stdout.
#[derive(Debug)]
pub struct NdjsonStdioTransport {
    stdin: tokio::sync::Mutex<BufReader<tokio::io::Stdin>>,
    stdout: tokio::sync::Mutex<tokio::io::Stdout>,
    closed: AtomicBool,
}

impl NdjsonStdioTransport {
    pub fn new() -> Self {
        Self {
            stdin: tokio::sync::Mutex::new(BufReader::new(tokio::io::stdin())),
            stdout: tokio::sync::Mutex::new(tokio::io::stdout()),
            closed: AtomicBool::new(false),
        }
    }
}

impl Default for NdjsonStdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Transport for NdjsonStdioTransport {
    async fn send(&mut self, message: TransportMessage) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(TransportError::ConnectionClosed.into());
        }

        let json_bytes = pmcp::shared::StdioTransport::serialize_message(&message)?;
        let mut stdout = self.stdout.lock().await;
        stdout.write_all(&json_bytes).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<TransportMessage> {
        if self.closed.load(Ordering::Acquire) {
            return Err(TransportError::ConnectionClosed.into());
        }

        let mut line = String::new();
        let bytes_read = {
            let mut stdin = self.stdin.lock().await;
            stdin.read_line(&mut line).await?
        };
        if bytes_read == 0 {
            self.closed.store(true, Ordering::Release);
            // pmcp's run_main_loop parks forever once the transport closes,
            // so without this the process hangs after EOF. The stdio MCP
            // contract is to exit when the client closes stdin; the grace
            // period lets any in-flight response finish flushing.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                std::process::exit(0);
            });
            return Err(TransportError::ConnectionClosed.into());
        }
        let line = line.trim();
        if line.is_empty() {
            // Blank separator lines carry no message; keep reading.
            return Box::pin(self.receive()).await;
        }
        pmcp::shared::StdioTransport::parse_message(line.as_bytes())
    }

    async fn close(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        let mut stdout = self.stdout.lock().await;
        stdout.flush().await.map_err(|e| e.into())
    }

    fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }

    fn transport_type(&self) -> &'static str {
        "stdio-ndjson"
    }
}
