//! # Research Master MCP
//!
//! A Model Context Protocol (MCP) server for searching and downloading academic papers
//! from multiple research sources.
//!
//! ## Architecture
//!
//! The library is organized into several modules:
//!
//! - [`models`]: Core data structures (Paper, SearchRequest, etc.)
//! - [`sources`]: Research source plugins with extensible trait-based architecture
//! - [`mcp`]: MCP protocol implementation and server
//! - [`utils`]: HTTP client, deduplication, and other utilities
//! - [`config`]: Configuration management

pub mod config;
pub mod mcp;
pub mod models;
pub mod sources;
pub mod ui;
pub mod utils;

// Re-export commonly used types
pub use models::Paper;
pub use sources::{Source, SourceRegistry};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    pub(crate) struct EnvSnapshot(Vec<(String, Option<std::ffi::OsString>)>);

    impl EnvSnapshot {
        pub(crate) fn new(vars: &[&str]) -> Self {
            Self(
                vars.iter()
                    .map(|var| (var.to_string(), std::env::var_os(var)))
                    .collect(),
            )
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (var, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(var, value),
                    None => std::env::remove_var(var),
                }
            }
        }
    }
}
