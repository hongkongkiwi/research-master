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
        let server = Self::build_server_impl(tools)?;
        Ok(Self {
            server: Arc::new(Mutex::new(server)),
        })
    }

    /// Get the tool registry
    pub fn tools(&self) -> Arc<Mutex<Server>> {
        self.server.clone()
    }

    /// Build the MCP server with tool handlers (internal implementation)
    fn build_server_impl(tools: ToolRegistry) -> Result<Server, pmcp::Error> {
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

        builder.build()
    }

    /// Run the server in stdio mode (for Claude Desktop and other MCP clients)
    pub async fn run(&self) -> Result<(), pmcp::Error> {
        tracing::info!("Starting MCP server in stdio mode");

        // run_stdio() takes ownership, so we need to extract the Server from Arc<Mutex>
        // Since we own the Arc and there should be no other references at this point,
        // we can try_unwrap to get the Server out
        let server = Arc::try_unwrap(self.server.clone())
            .map_err(|_| Error::internal("Cannot unwrap Arc - multiple references exist"))?
            .into_inner();

        tracing::info!("MCP server initialized");

        server.run_stdio().await
    }

    /// Run the server in HTTP/SSE mode
    ///
    /// This starts an HTTP server that uses Server-Sent Events (SSE) for real-time
    /// communication with MCP clients.
    pub async fn run_http(&self, addr: &str) -> Result<(SocketAddr, JoinHandle<()>), pmcp::Error> {
        tracing::info!("Starting MCP server in HTTP/SSE mode on {}", addr);

        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| Error::invalid_params(format!("Invalid address: {}", e)))?;

        // Create the HTTP server with default config
        let http_server = StreamableHttpServer::new(socket_addr, self.server.clone());

        // Start the server
        http_server.start().await
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
