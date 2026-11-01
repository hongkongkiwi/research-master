//! MCP handlers for section outlines, section reads, and arXiv LaTeX sources.

use std::sync::Arc;

use serde_json::Value;

use super::tools::ToolHandler;
use crate::models::ReadRequest;
use crate::sources::Source;
use crate::utils::sections::{SectionOutline, SectionSpan};

/// Default per-section character budget for read_paper_section.
const DEFAULT_SECTION_MAX_CHARS: u64 = 12000;

/// Preview length per section in get_paper_outline.
const OUTLINE_PREVIEW_CHARS: usize = 120;

/// Outline of a paper's extracted text: heading spans with previews.
#[derive(Debug)]
pub struct GetPaperOutlineHandler {
    pub sources: Arc<Vec<Arc<dyn Source>>>,
}

/// Read one detected section of a paper's text.
#[derive(Debug)]
pub struct ReadPaperSectionHandler {
    pub sources: Arc<Vec<Arc<dyn Source>>>,
}

/// Fetch the LaTeX source bundle of an arXiv paper.
#[derive(Debug)]
pub struct GetPaperLatexHandler;

/// Prefer arXiv for reading, falling back to whatever is registered.
fn find_source(
    sources: &[Arc<dyn Source>],
    source_override: Option<&str>,
) -> Result<Arc<dyn Source>, String> {
    if let Some(id) = source_override {
        return sources
            .iter()
            .find(|s| s.id() == id)
            .cloned()
            .ok_or_else(|| format!("Source '{}' not found", id));
    }
    sources
        .iter()
        .find(|s| s.id() == "arxiv")
        .or_else(|| sources.first())
        .cloned()
        .ok_or_else(|| "No sources available".to_string())
}

async fn read_paper(source: &Arc<dyn Source>, paper_id: &str) -> Result<String, String> {
    let request = ReadRequest::new(paper_id, "./downloads");
    let result = source.read(&request).await.map_err(|e| e.to_string())?;
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| format!("Failed to read paper '{paper_id}'")));
    }
    Ok(result.text)
}

fn outline_response(text: &str) -> Value {
    let outline = crate::utils::sections::outline(text);
    let sections: Vec<Value> = outline
        .sections
        .iter()
        .map(|span| {
            serde_json::json!({
                "title": span.title,
                "start": span.start,
                "end": span.end,
                "preview": crate::utils::sections::read_section(text, span, OUTLINE_PREVIEW_CHARS),
            })
        })
        .collect();
    serde_json::json!({ "sections": sections })
}

fn span_by_index(outline: &SectionOutline, index: usize) -> Result<&SectionSpan, String> {
    outline.sections.get(index).ok_or_else(|| {
        format!(
            "Section index {index} out of range ({} sections)",
            outline.sections.len()
        )
    })
}

/// Resolve the `section` argument: 0-based index (JSON number or numeric
/// string) or exact section title.
fn select_span<'a>(
    outline: &'a SectionOutline,
    section: &Value,
) -> Result<&'a SectionSpan, String> {
    if let Some(n) = section.as_u64() {
        return span_by_index(outline, n as usize);
    }
    let value = section
        .as_str()
        .ok_or("'section' must be an index or a title")?;
    if let Ok(index) = value.parse::<usize>() {
        return span_by_index(outline, index);
    }
    outline
        .sections
        .iter()
        .find(|span| span.title == value)
        .ok_or_else(|| format!("Section '{value}' not found"))
}

fn section_response(text: &str, span: &SectionSpan, max_chars: usize) -> Value {
    let out = crate::utils::sections::read_section(text, span, max_chars);
    let total_chars = text
        .get(span.start..span.end)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let truncated = out.chars().count() < total_chars;
    let next_start = truncated.then(|| span.start + out.len());
    serde_json::json!({
        "title": span.title,
        "text": out,
        "truncated": truncated,
        "next_start": next_start,
    })
}

#[async_trait::async_trait]
impl ToolHandler for GetPaperOutlineHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let source = find_source(&self.sources, source_override)?;
        let text = read_paper(&source, paper_id).await?;
        Ok(outline_response(&text))
    }
}

#[async_trait::async_trait]
impl ToolHandler for ReadPaperSectionHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let section = args.get("section").ok_or("Missing 'section' parameter")?;
        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_SECTION_MAX_CHARS) as usize;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let source = find_source(&self.sources, source_override)?;
        let text = read_paper(&source, paper_id).await?;
        let outline = crate::utils::sections::outline(&text);
        let span = select_span(&outline, section)?;
        Ok(section_response(&text, span, max_chars))
    }
}

#[async_trait::async_trait]
impl ToolHandler for GetPaperLatexHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let arxiv_id = args
            .get("arxiv_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'arxiv_id' parameter")?;
        let latex = crate::utils::latex::fetch_arxiv_latex(arxiv_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "arxiv_id": arxiv_id,
            "files": latex.files,
            "main_tex": latex.main_tex,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_text() -> String {
        String::from(
            "We study attention models for machine translation.\n\n\
             1 Introduction\n\
             Sequence models dominate modern NLP work.\n\n\
             2 Methods\n\
             We use transformer layers throughout.\n",
        )
    }

    #[test]
    fn outline_response_includes_previews() {
        let value = outline_response(&sample_text());
        let sections = value["sections"].as_array().expect("sections array");
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0]["title"], "Front matter");
        let preview = sections[1]["preview"].as_str().expect("preview");
        assert!(preview.starts_with("1 Introduction"));
        assert!(preview.chars().count() <= OUTLINE_PREVIEW_CHARS);
    }

    #[test]
    fn select_span_by_index_and_exact_title() {
        let outline = crate::utils::sections::outline(&sample_text());
        let by_index = select_span(&outline, &serde_json::json!(1)).expect("index");
        assert_eq!(by_index.title, "1 Introduction");
        let by_title = select_span(&outline, &serde_json::json!("2 Methods")).expect("title");
        assert_eq!(by_title.title, "2 Methods");
    }

    #[test]
    fn select_span_rejects_out_of_range_and_unknown_title() {
        let outline = crate::utils::sections::outline(&sample_text());
        assert!(select_span(&outline, &serde_json::json!(99)).is_err());
        assert!(select_span(&outline, &serde_json::json!("No such section")).is_err());
    }

    #[test]
    fn section_response_truncates_and_reports_next_start() {
        let text = sample_text();
        let outline = crate::utils::sections::outline(&text);
        let span = select_span(&outline, &serde_json::json!("1 Introduction")).expect("span");

        let truncated_value = section_response(&text, span, 10);
        assert_eq!(truncated_value["truncated"], true);
        let out = truncated_value["text"].as_str().expect("text");
        assert_eq!(out.chars().count(), 10);
        assert_eq!(
            truncated_value["next_start"].as_u64().expect("next_start") as usize,
            span.start + out.len()
        );

        let full = section_response(&text, span, 10_000);
        assert_eq!(full["truncated"], false);
        assert!(full["next_start"].is_null());
    }

    #[tokio::test]
    async fn handlers_reject_missing_arguments() {
        let sources: Arc<Vec<Arc<dyn Source>>> = Arc::new(Vec::new());

        let outline_handler = GetPaperOutlineHandler {
            sources: Arc::clone(&sources),
        };
        assert!(outline_handler
            .execute(serde_json::json!({}))
            .await
            .is_err());

        let read_handler = ReadPaperSectionHandler { sources };
        assert!(read_handler
            .execute(serde_json::json!({ "paper_id": "2301.00001" }))
            .await
            .is_err());
        assert!(read_handler
            .execute(serde_json::json!({ "section": 0 }))
            .await
            .is_err());

        let latex_handler = GetPaperLatexHandler;
        assert!(latex_handler.execute(serde_json::json!({})).await.is_err());
    }
}
