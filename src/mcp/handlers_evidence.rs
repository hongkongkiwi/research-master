//! MCP handlers for evidence tools.
//!
//! Provides [`GetSmartCitationsHandler`], the handler behind the
//! `get_citation_tallies` tool: Scite smart-citation tallies
//! (supporting/contradicting/mentioning) per DOI, plus retraction status
//! from the OpenAlex single-work lookup.

use std::sync::Arc;

use serde_json::{json, Value};

use super::tools::{Tool, ToolHandler};
use crate::utils::evidence::{
    check_retraction_from, scite_tallies_from, OPENALEX_WORKS_URL, SCITE_TALLIES_URL,
};
use crate::utils::HttpClient;

/// Handler for `get_citation_tallies`.
///
/// Accepts `doi` (string) or `dois` (array of strings, takes precedence) and
/// optional `include_retraction` (default true). Each DOI produces an entry
/// with `tallies` and `retraction` sub-objects; failures surface as
/// `{"error": ...}` inside the affected sub-object, never as a tool-level
/// error, so one bad DOI does not sink a batch.
#[derive(Debug)]
pub struct GetSmartCitationsHandler;

#[async_trait::async_trait]
impl ToolHandler for GetSmartCitationsHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let dois = parse_dois(&args)?;
        let include_retraction = args
            .get("include_retraction")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let client =
            HttpClient::new().map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let mut results = Vec::with_capacity(dois.len());
        for doi in &dois {
            results.push(
                evidence_entry(
                    &client,
                    SCITE_TALLIES_URL,
                    OPENALEX_WORKS_URL,
                    doi,
                    include_retraction,
                )
                .await,
            );
        }

        Ok(json!({
            "tool": "get_citation_tallies",
            "count": results.len(),
            "results": results,
        }))
    }
}

/// Extract the DOI list: `dois` (array) takes precedence over `doi` (single).
fn parse_dois(args: &Value) -> Result<Vec<String>, String> {
    if let Some(list) = args.get("dois").and_then(Value::as_array) {
        let dois: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if dois.is_empty() {
            return Err("'dois' must contain at least one DOI string".to_string());
        }
        return Ok(dois);
    }

    if let Some(doi) = args.get("doi").and_then(Value::as_str) {
        let doi = doi.trim();
        if doi.is_empty() {
            return Err("'doi' must not be empty".to_string());
        }
        return Ok(vec![doi.to_string()]);
    }

    Err("Missing 'doi' (string) or 'dois' (array of strings) parameter".to_string())
}

/// Build one result entry: Scite tallies plus optional OpenAlex retraction
/// status. Base URLs are parameters so tests can point at a mockito server.
async fn evidence_entry(
    client: &HttpClient,
    scite_base: &str,
    openalex_base: &str,
    doi: &str,
    include_retraction: bool,
) -> Value {
    let tallies = match scite_tallies_from(scite_base, client, doi).await {
        Ok(t) => json!(t),
        Err(e) => json!({ "error": e.to_string() }),
    };

    let retraction = if include_retraction {
        match check_retraction_from(openalex_base, client, doi).await {
            Ok(info) => json!(info),
            Err(e) => json!({ "error": e.to_string() }),
        }
    } else {
        Value::Null
    };

    json!({
        "doi": doi,
        "tallies": tallies,
        "retraction": retraction,
    })
}

/// Build the `get_citation_tallies` tool for registry registration.
pub fn get_citation_tallies_tool() -> Tool {
    Tool {
        name: "get_citation_tallies".to_string(),
        description: "Get Scite smart-citation tallies (supporting, contradicting, mentioning) for one or more DOIs, with retraction status from OpenAlex.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "doi": {
                    "type": "string",
                    "description": "Single DOI (e.g., '10.1038/nature12373')"
                },
                "dois": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Batch of DOIs; takes precedence over 'doi'"
                },
                "include_retraction": {
                    "type": "boolean",
                    "description": "Include retraction status from OpenAlex",
                    "default": true
                }
            }
        }),
        handler: Arc::new(GetSmartCitationsHandler),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Build a client with proxy env vars cleared so requests reach the
    /// local mockito server instead of any ambient proxy.
    fn test_client() -> HttpClient {
        let guard = env_lock().lock().expect("env lock poisoned");
        const VARS: [&str; 6] = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&str, Option<String>)> =
            VARS.iter().map(|v| (*v, std::env::var(v).ok())).collect();
        for var in VARS {
            std::env::remove_var(var);
        }
        let client =
            HttpClient::without_rate_limit("research-master-tests").expect("test HTTP client");
        for (var, value) in previous {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        drop(guard);
        client
    }

    #[test]
    fn test_parse_dois_variants() {
        assert_eq!(
            parse_dois(&json!({"dois": [" 10.1/A ", "", "10.2/B"]})).unwrap(),
            vec!["10.1/A".to_string(), "10.2/B".to_string()]
        );
        assert_eq!(
            parse_dois(&json!({"doi": " 10.1038/nature12373 "})).unwrap(),
            vec!["10.1038/nature12373".to_string()]
        );
        // 'dois' takes precedence over 'doi'
        assert_eq!(
            parse_dois(&json!({"dois": ["10.1/A"], "doi": "10.2/B"})).unwrap(),
            vec!["10.1/A".to_string()]
        );
        assert!(parse_dois(&json!({})).is_err());
        assert!(parse_dois(&json!({"dois": []})).is_err());
        assert!(parse_dois(&json!({"dois": ["", "  "]})).is_err());
        assert!(parse_dois(&json!({"doi": "   "})).is_err());
        assert!(parse_dois(&json!({"doi": 42})).is_err());
    }

    #[tokio::test]
    async fn test_evidence_entry_combines_tallies_and_retraction() {
        let mut server = mockito::Server::new_async().await;
        let tallies_mock = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"total":1937,"supporting":22,"contradicting":1,"mentioning":1905,
                    "unclassified":9,"doi":"10.1038/nature12373","citingPublications":2163}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let openalex_mock = server
            .mock("GET", "/works/doi:10.1038%2Fnature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W2159974629","is_retracted":false,
                    "retraction_date":null,
                    "title":"Nanometre-scale thermometry in a living cell"}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let client = test_client();
        let entry = evidence_entry(
            &client,
            &format!("{}/tallies/", server.url()),
            &format!("{}/works/doi:", server.url()),
            "10.1038/nature12373",
            true,
        )
        .await;

        assert_eq!(entry["doi"], "10.1038/nature12373");
        assert_eq!(entry["tallies"]["supporting"], 22);
        assert_eq!(entry["tallies"]["contradicting"], 1);
        assert_eq!(entry["tallies"]["citingPublications"], 2163);
        assert_eq!(entry["retraction"]["is_retracted"], false);
        assert_eq!(
            entry["retraction"]["openalex_id"],
            "https://openalex.org/W2159974629"
        );
        assert!(entry["tallies"].get("error").is_none());
        assert!(entry["retraction"].get("error").is_none());

        tallies_mock.assert_async().await;
        openalex_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_evidence_entry_scite_error_keeps_retraction() {
        let mut server = mockito::Server::new_async().await;
        let tallies_mock = server
            .mock("GET", "/tallies/10.9999/bogus")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"detail":"Tally not found"}"#)
            .expect(1)
            .create_async()
            .await;
        let openalex_mock = server
            .mock("GET", "/works/doi:10.9999%2Fbogus")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"https://openalex.org/W1","is_retracted":true}"#)
            .expect(1)
            .create_async()
            .await;

        let client = test_client();
        let entry = evidence_entry(
            &client,
            &format!("{}/tallies/", server.url()),
            &format!("{}/works/doi:", server.url()),
            "10.9999/bogus",
            true,
        )
        .await;

        assert!(entry["tallies"]["error"].is_string(), "got {}", entry);
        assert_eq!(entry["retraction"]["is_retracted"], true);

        tallies_mock.assert_async().await;
        openalex_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_evidence_entry_without_retraction_lookup() {
        let mut server = mockito::Server::new_async().await;
        let tallies_mock = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"total":2,"supporting":1,"contradicting":0,"mentioning":1,
                    "unclassified":0,"doi":"10.1038/nature12373"}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let client = test_client();
        let entry = evidence_entry(
            &client,
            &format!("{}/tallies/", server.url()),
            &format!("{}/works/doi:", server.url()),
            "10.1038/nature12373",
            false,
        )
        .await;

        assert_eq!(entry["tallies"]["total"], 2);
        assert!(entry["retraction"].is_null(), "got {}", entry);
        // A regression that still calls OpenAlex would get mockito's 501 and
        // surface {"error": ...} here instead of null.
        assert!(entry["retraction"].get("error").is_none());

        tallies_mock.assert_async().await;
    }

    #[test]
    fn test_get_citation_tallies_tool_shape() {
        let tool = get_citation_tallies_tool();
        assert_eq!(tool.name, "get_citation_tallies");
        let properties = tool.input_schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("doi"));
        assert!(properties.contains_key("dois"));
        assert!(properties.contains_key("include_retraction"));
    }
}
