//! MCP (Model Context Protocol) implementation.
//!
//! This module provides the MCP server implementation using the [pmcp] crate
//! for proper JSON-RPC handling over stdio and HTTP/SSE.
//!
//! # Components
//!
//! - [`McpServer`]: Main MCP server that can run in stdio or HTTP/SSE mode
//! - [`ToolRegistry`]: Registry of available MCP tools
//! - [`Tool`]: Tool descriptor with name, description, and handler
//!
//! # Server Modes
//!
//! ## stdio Mode
//!
//! For integration with Claude Desktop and other MCP clients that communicate
//! over standard input/output:
//!
//! ```rust,no_run
//! use research_master::{mcp::McpServer, sources::SourceRegistry};
//! use std::sync::Arc;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let sources = Arc::new(SourceRegistry::new());
//! let server = McpServer::new(sources)?;
//! server.run().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## HTTP/SSE Mode
//!
//! For web-based clients using Server-Sent Events:
//!
//! ```rust,no_run
//! use research_master::{mcp::McpServer, sources::SourceRegistry};
//! use std::sync::Arc;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let sources = Arc::new(SourceRegistry::new());
//! let server = McpServer::new(sources)?;
//! let (addr, handle) = server.run_http("127.0.0.1:3000").await?;
//! # Ok(())
//! # }
//! ```
//!
//! [pmcp]: https://docs.rs/pmcp

pub mod handlers_evidence;
pub mod handlers_grobid;
pub mod handlers_library;
pub mod handlers_reading;
pub mod handlers_watch;
pub mod http_auth;
pub mod prompts;
pub mod server;
pub mod sse;
mod tools;
pub mod unified_tools;

pub use server::McpServer;
pub use tools::{Tool, ToolRegistry};
