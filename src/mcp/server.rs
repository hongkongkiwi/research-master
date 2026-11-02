//! MCP server implementation using pmcp (Pragmatic AI's rust-mcp-sdk).
//!
//! This module provides the MCP server implementation using the pmcp crate
//! for proper JSON-RPC handling over stdio and HTTP/SSE.

use crate::mcp::tools::ToolRegistry;
use crate::sources::SourceRegistry;
use async_trait::async_trait;
use pmcp::{
    server::streamable_http_server::{StreamableHttpServer, StreamableHttpServerConfig},
    Error, RequestHandlerExtra, Server, ServerCapabilities, ToolHandler, ToolInfo,
};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// The MCP server for Research Master
///
/// This server provides tools for searching and downloading academic papers
/// from multiple research sources over various transports.
#[derive(Debug, Clone)]
pub struct McpServer {
    server: Arc<Mutex<Server>>,
    /// Kept so network transports can build a fresh `Server` per client
    /// session with clean initialization state.
    tools: ToolRegistry,
}

impl McpServer {
    /// Create a new MCP server with the given source registry
    pub fn new(sources: Arc<SourceRegistry>) -> Result<Self, pmcp::Error> {
        if sources.is_empty() {
            return Err(Error::invalid_params(
                "Cannot create MCP server without any registered sources",
            ));
        }

        let tools = ToolRegistry::from_sources(&sources);
        let server = Self::build_server_impl(tools.clone())?;
        Ok(Self {
            server: Arc::new(Mutex::new(server)),
            tools,
        })
    }

    /// Get the tool registry
    pub fn tools(&self) -> Arc<Mutex<Server>> {
        self.server.clone()
    }

    /// Build the MCP server with tool handlers (internal implementation)
    pub(crate) fn build_server_impl(tools: ToolRegistry) -> Result<Server, pmcp::Error> {
        let mut builder = Server::builder()
            .name("research-master")
            .version(env!("CARGO_PKG_VERSION"))
            .capabilities(ServerCapabilities::default());

        // Add all tools from the registry
        for tool in tools.all() {
            let name = tool.name.clone();
            let description = tool.description.clone();
            let input_schema = tool.input_schema.clone();
            let handler = tool.handler.clone();

            let tool_handler = ToolWrapper {
                name,
                description: Some(description),
                input_schema,
                handler,
            };
            builder = builder.tool(tool_handler.name.clone(), tool_handler);
        }

        // Built-in prompts: context scaffolding that guides agents through the
        // multi-tool flows each prompt encodes.
        builder = builder
            .prompt(
                "literature_review",
                crate::mcp::prompts::LiteratureReviewPrompt,
            )
            .prompt(
                "deep_paper_analysis",
                crate::mcp::prompts::DeepPaperAnalysisPrompt,
            )
            .prompt(
                "research_discovery",
                crate::mcp::prompts::ResearchDiscoveryPrompt,
            )
            .prompt("citation_audit", crate::mcp::prompts::CitationAuditPrompt)
            .prompt(
                "systematic_review_screening",
                crate::mcp::prompts::SystematicReviewScreeningPrompt,
            );

        builder.build()
    }

    /// Run the server in stdio mode (for Claude Desktop and other MCP clients)
    ///
    /// Uses newline-delimited JSON-RPC framing per the MCP stdio transport
    /// spec; pmcp 1.9.4's built-in stdio transport speaks LSP-style
    /// Content-Length framing, which standard MCP clients do not send.
    /// Consumes the server: `run` takes ownership of the inner `Server`.
    pub async fn run(self) -> Result<(), pmcp::Error> {
        tracing::info!("Starting MCP server in stdio mode");
        let server = Arc::try_unwrap(self.server)
            .map_err(|_| Error::internal("Cannot unwrap Arc - multiple references exist"))?
            .into_inner();

        tracing::info!("MCP server initialized");

        server
            .run(crate::mcp::stdio_transport::NdjsonStdioTransport::new())
            .await
    }

    /// Run the server in HTTP/SSE mode
    ///
    /// This starts an HTTP server that uses Server-Sent Events (SSE) for real-time
    /// communication with MCP clients. When `RESEARCH_MASTER_HTTP_TOKEN` is set,
    /// requests must present that bearer token or they are rejected.
    pub async fn run_http(&self, addr: &str) -> Result<(SocketAddr, JoinHandle<()>), pmcp::Error> {
        let config = StreamableHttpServerConfig {
            http_middleware: Some(std::sync::Arc::new(
                crate::mcp::http_auth::BearerTokenMiddleware::chain_from_env().unwrap_or_default(),
            )),
            // No event store: pmcp's GET replay path sits outside the bearer
            // middleware and replays stored responses on an unknown
            // Last-Event-ID. Without a store there is nothing to replay.
            event_store: None,
            ..Default::default()
        };
        self.run_http_with_config(addr, config).await
    }

    /// Run the server in HTTP/SSE mode with custom configuration
    pub async fn run_http_with_config(
        &self,
        addr: &str,
        config: StreamableHttpServerConfig,
    ) -> Result<(SocketAddr, JoinHandle<()>), pmcp::Error> {
        tracing::info!(
            "Starting MCP server in HTTP/SSE mode on {} (with custom config)",
            addr
        );

        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| Error::invalid_params(format!("Invalid address: {}", e)))?;

        // Create the HTTP server with custom config
        let http_server =
            StreamableHttpServer::with_config(socket_addr, self.server.clone(), config);

        // Start the server
        http_server.start().await
    }

    /// Run the server with the legacy HTTP + SSE transport (spec 2024-11-05)
    ///
    /// Serves `GET /sse` (Server-Sent Events) plus `POST /messages?sessionId=`.
    /// Bearer auth follows the streamable HTTP transport: set
    /// `RESEARCH_MASTER_HTTP_TOKEN` to require `Authorization: Bearer <token>`.
    pub async fn run_sse(&self, addr: &str) -> Result<(SocketAddr, JoinHandle<()>), pmcp::Error> {
        tracing::info!("Starting MCP server in legacy SSE mode on {}", addr);
        crate::mcp::sse::run(self.tools.clone(), addr).await
    }
}

/// Wrapper for adapting our Tool to pmcp's ToolHandler
#[derive(Clone)]
struct ToolWrapper {
    name: String,
    description: Option<String>,
    input_schema: Value,
    handler: Arc<dyn crate::mcp::tools::ToolHandler>,
}

#[async_trait]
impl ToolHandler for ToolWrapper {
    async fn handle(&self, args: Value, _extra: RequestHandlerExtra) -> Result<Value, Error> {
        self.handler
            .execute(args)
            .await
            .map_err(|e| Error::internal(&e))
    }

    fn metadata(&self) -> Option<ToolInfo> {
        Some(ToolInfo::new(
            self.name.clone(),
            self.description.clone(),
            self.input_schema.clone(),
        ))
    }
}

/// Create a new MCP server instance
pub fn create_mcp_server(sources: Arc<SourceRegistry>) -> Result<McpServer, pmcp::Error> {
    McpServer::new(sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{MockSource, SourceRegistry};

    fn source_registry_with_mock() -> Arc<SourceRegistry> {
        let mut registry = SourceRegistry::empty_for_tests();
        registry.register(Arc::new(MockSource::new()));
        Arc::new(registry)
    }

    #[test]
    fn test_server_creation_succeeds_with_source_registry() {
        let server = McpServer::new(source_registry_with_mock());
        assert!(
            server.is_ok(),
            "server should build with at least one source"
        );
    }

    #[test]
    fn test_server_creation_fails_with_empty_source_registry() {
        let sources = Arc::new(SourceRegistry::empty_for_tests());
        let server = McpServer::new(sources);
        assert!(
            server.is_err(),
            "server should reject an empty source registry"
        );
    }

    #[test]
    fn test_tools_returns_arc_mutex_server() {
        let server = McpServer::new(source_registry_with_mock()).expect("server should build");
        let tools = server.tools();

        assert_eq!(Arc::strong_count(&tools), 2);
        assert!(
            tools.try_lock().is_ok(),
            "returned Arc should contain a lockable Server"
        );
    }
}
