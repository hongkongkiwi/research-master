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

async fn find_source(
    sources: &Arc<Vec<Arc<dyn Source>>>,
    paper_id: &str,
    source_override: Option<&str>,
) -> Result<(Arc<dyn Source>, String), String> {
    if let Some(id) = source_override {
        return sources
            .iter()
            .find(|s| s.id() == id)
            .cloned()
            .map(|s| (s, paper_id.to_string()))
            .ok_or_else(|| format!("Source '{}' not found", id));
    }
    let readable: Arc<Vec<Arc<dyn Source>>> = Arc::new(
        sources
            .iter()
            .filter(|s| s.supports_read() || s.supports_download())
            .cloned()
            .collect(),
    );
    if paper_id.starts_with("10.") {
        for source in readable.iter().filter(|s| s.supports_doi_lookup()) {
            if let Ok(paper) = source.get_by_doi(paper_id).await {
                return Ok((Arc::clone(source), paper.paper_id));
            }
        }
        return Err(
            "Could not resolve DOI to a readable source; specify source explicitly".to_string(),
        );
    }
    let source = super::unified_tools::auto_detect_source(&readable, paper_id)?;
    let id = paper_id
        .strip_prefix("arXiv:")
        .or_else(|| paper_id.strip_prefix("arxiv:"))
        .unwrap_or(paper_id);
    if source.id() == "arxiv"
        && !(paper_id.to_ascii_lowercase().starts_with("arxiv:")
            || id.split_once('.').is_some_and(|(a, b)| {
                a.len() == 4
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.split('v')
                        .next()
                        .is_some_and(|n| n.len() >= 4 && n.bytes().all(|c| c.is_ascii_digit()))
            })
            || id.split_once('/').is_some_and(|(a, b)| {
                !a.is_empty()
                    && a.bytes().all(|c| c.is_ascii_alphabetic() || c == b'-')
                    && b.len() >= 7
                    && b.bytes().all(|c| c.is_ascii_digit())
            }))
    {
        return Err(
            "Could not auto-detect a readable source; specify source explicitly".to_string(),
        );
    }
    Ok((source, paper_id.to_string()))
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

fn section_response(
    text: &str,
    span: &SectionSpan,
    offset: usize,
    max_chars: usize,
) -> Result<Value, String> {
    let section = &text[span.start..span.end];
    let remaining = section
        .get(offset..)
        .ok_or("offset must be a byte boundary within the section")?;
    let out: String = remaining.chars().take(max_chars).collect();
    let next = offset + out.len();
    let truncated = next < section.len();
    Ok(serde_json::json!({
        "title": span.title,
        "text": out,
        "truncated": truncated,
        "next_start": truncated.then_some(next),
        "offset_units": "bytes",
    }))
}

#[async_trait::async_trait]
impl ToolHandler for GetPaperOutlineHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let (source, resolved_id) = find_source(&self.sources, paper_id, source_override).await?;
        let text = read_paper(&source, &resolved_id).await?;
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
        let (source, resolved_id) = find_source(&self.sources, paper_id, source_override).await?;
        let text = read_paper(&source, &resolved_id).await?;
        let outline = crate::utils::sections::outline(&text);
        let span = select_span(&outline, section)?;
        let offset = match args.get("offset") {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("'offset' must be a non-negative integer")?,
        };
        if max_chars == 0 {
            return Err("'max_chars' must be greater than zero".to_string());
        }
        section_response(&text, span, offset, max_chars)
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

        let truncated_value = section_response(&text, span, 0, 10).unwrap();
        assert_eq!(truncated_value["truncated"], true);
        let out = truncated_value["text"].as_str().expect("text");
        assert_eq!(out.chars().count(), 10);
        assert_eq!(
            truncated_value["next_start"].as_u64().expect("next_start") as usize,
            out.len()
        );

        let full = section_response(&text, span, 0, 10_000).unwrap();
        assert_eq!(full["truncated"], false);
        assert!(full["next_start"].is_null());
    }

    #[test]
    fn section_byte_offsets_continue_multibyte_text() {
        let text = "prefixéλx";
        let span = SectionSpan {
            title: "test".into(),
            start: 6,
            end: text.len(),
        };
        let first = section_response(text, &span, 0, 1).unwrap();
        assert_eq!(first["text"], "é");
        assert_eq!(first["next_start"], 2);
        let second = section_response(text, &span, 2, 1).unwrap();
        assert_eq!(second["text"], "λ");
        assert_eq!(second["next_start"], 4);
        let last = section_response(text, &span, 4, 1).unwrap();
        assert_eq!(last["text"], "x");
        assert_eq!(last["truncated"], false);
        assert!(section_response(text, &span, 1, 1).is_err());
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
