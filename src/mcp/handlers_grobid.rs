//! MCP tool handlers for GROBID PDF extraction: reference lists and the
//! header record of a local PDF, via the self-hosted GROBID service.

use std::path::Path;

use serde_json::{json, Value};

use super::tools::ToolHandler;
use crate::utils::grobid::{extract_header, extract_references};
use crate::utils::HttpClient;

/// Appended to handler errors so callers know how to bring GROBID up.
const SETUP_HINT: &str = "GROBID service required: set GROBID_URL (default \
http://localhost:8070) and start one, e.g. docker run --rm -p 8070:8070 \
lfoppiano/grobid:0.8.0";

fn require_pdf_path(args: &Value) -> Result<String, String> {
    args.get("pdf_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing 'pdf_path' parameter. {SETUP_HINT}"))
}

fn client_with_hint() -> Result<HttpClient, String> {
    HttpClient::new().map_err(|e| format!("Failed to create HTTP client: {e}. {SETUP_HINT}"))
}

/// Extract a PDF's reference list as papers, in citation order.
#[derive(Debug)]
pub struct ExtractReferencesHandler;

#[async_trait::async_trait]
impl ToolHandler for ExtractReferencesHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let pdf_path = require_pdf_path(&args)?;
        let client = client_with_hint()?;
        let papers = extract_references(&client, Path::new(&pdf_path))
            .await
            .map_err(|e| format!("{e}. {SETUP_HINT}"))?;
        let count = papers.len();
        let papers =
            serde_json::to_value(&papers).map_err(|e| format!("cannot serialize papers: {e}"))?;
        Ok(json!({
            "count": count,
            "papers": papers,
        }))
    }
}

/// Extract a PDF's header (title, authors, year, DOI, abstract) as one paper.
#[derive(Debug)]
pub struct ExtractPaperHeaderHandler;

#[async_trait::async_trait]
impl ToolHandler for ExtractPaperHeaderHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let pdf_path = require_pdf_path(&args)?;
        let client = client_with_hint()?;
        let paper = extract_header(&client, Path::new(&pdf_path))
            .await
            .map_err(|e| format!("{e}. {SETUP_HINT}"))?;
        serde_json::to_value(&paper).map_err(|e| format!("cannot serialize paper: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes GROBID_URL and proxy env access across tests.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Restores the saved env values when dropped at test end.
    struct EnvGuard(Vec<(&'static str, Option<String>)>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    /// Point GROBID_URL at one test's mock server and clear proxies so the
    /// handler's own HttpClient reaches it directly; hold both guards until
    /// the test ends.
    async fn scoped_grobid_env(url: &str) -> (tokio::sync::MutexGuard<'static, ()>, EnvGuard) {
        let lock = ENV_LOCK.lock().await;
        const VARS: [&str; 7] = [
            "GROBID_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&'static str, Option<String>)> = VARS
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in VARS {
            std::env::remove_var(name);
        }
        std::env::set_var("GROBID_URL", url);
        (lock, EnvGuard(previous))
    }

    fn temp_pdf(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "research-master-{name}-{}-grobid-handler-test.pdf",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("write temp PDF");
        path
    }

    const HEADER_TEI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<TEI xmlns="http://www.tei-c.org/ns/1.0">
  <teiHeader>
    <fileDesc>
      <titleStmt>
        <title level="a" type="main">Attention Is All You Need</title>
        <author>
          <persName><forename type="first">Ashish</forename><surname>Vaswani</surname></persName>
        </author>
      </titleStmt>
      <publicationStmt>
        <date type="publication" when="2017-06-12">June 2017</date>
        <idno type="DOI">10.48550/arXiv.1706.03762</idno>
      </publicationStmt>
    </fileDesc>
    <profileDesc>
      <abstract><p>Sequence transduction models based on attention.</p></abstract>
    </profileDesc>
  </teiHeader>
</TEI>"#;

    #[tokio::test]
    async fn missing_pdf_path_arg_errors_with_hint() {
        let err = ExtractReferencesHandler
            .execute(json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("pdf_path"), "got: {err}");
        assert!(err.contains("docker run"), "got: {err}");

        let err = ExtractPaperHeaderHandler
            .execute(json!({ "pdf_path": "   " }))
            .await
            .unwrap_err();
        assert!(err.contains("pdf_path"), "got: {err}");
    }

    #[tokio::test]
    async fn nonexistent_pdf_path_errors_before_network() {
        let missing = std::env::temp_dir().join(format!(
            "research-master-missing-{}-grobid-handler-test.pdf",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);

        let err = ExtractReferencesHandler
            .execute(json!({ "pdf_path": missing.to_string_lossy() }))
            .await
            .unwrap_err();
        assert!(err.contains("cannot read PDF"), "got: {err}");
        assert!(err.contains("docker run"), "got: {err}");
    }

    #[tokio::test]
    async fn header_handler_returns_paper_json() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/processHeaderDocument?consolidateHeader=1")
            .match_header("accept", "application/tei+xml")
            .with_status(200)
            .with_header("content-type", "application/tei+xml")
            .with_body(HEADER_TEI)
            .expect(1)
            .create_async()
            .await;

        let (_lock, _env) = scoped_grobid_env(&server.url()).await;
        let path = temp_pdf("header", b"%PDF-1.4");
        let paper = ExtractPaperHeaderHandler
            .execute(json!({ "pdf_path": path.to_string_lossy() }))
            .await
            .expect("paper JSON");
        let _ = std::fs::remove_file(&path);

        assert_eq!(paper["title"], "Attention Is All You Need");
        assert_eq!(paper["paper_id"], "10.48550/arXiv.1706.03762");
        assert_eq!(paper["doi"], "10.48550/arXiv.1706.03762");
        assert_eq!(paper["published_date"], "2017");
        mock.assert_async().await;
    }
}
