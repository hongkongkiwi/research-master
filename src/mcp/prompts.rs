//! Built-in MCP prompts steering agents through multi-tool research flows.
//!
//! Prompts are context scaffolding: each returns a user message with the exact
//! tool sequence that produces a good result for the stated task, so clients
//! can discover correct usage without reading tool docs.

use pmcp::server::cancellation::RequestHandlerExtra;
use pmcp::server::PromptHandler;
use pmcp::types::{Content, GetPromptResult, PromptMessage, Role};
use pmcp::{Error, Result};
use std::collections::HashMap;

fn prompt(
    description: &str,
    args: &HashMap<String, String>,
    key: &str,
    default: &str,
    body: impl FnOnce(&str) -> String,
) -> Result<GetPromptResult> {
    let value = args
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.to_string());
    Ok(GetPromptResult {
        description: Some(description.to_string()),
        messages: vec![PromptMessage {
            role: Role::User,
            content: Content::Text { text: body(&value) },
        }],
    })
}

fn arg(args: &HashMap<String, String>, key: &str, default: &str) -> String {
    args.get(key)
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

/// Step-by-step literature review over the search, read and export tools.
#[derive(Debug, Default)]
pub struct LiteratureReviewPrompt;

#[async_trait::async_trait]
impl PromptHandler for LiteratureReviewPrompt {
    async fn handle(
        &self,
        args: HashMap<String, String>,
        _extra: RequestHandlerExtra,
    ) -> Result<GetPromptResult> {
        let topic = arg(&args, "topic", "");
        if topic.trim().is_empty() {
            return Err(Error::invalid_params("Missing 'topic' argument"));
        }
        let max_papers = arg(&args, "max_papers", "10");
        prompt(
            "Review the literature on a topic using search, focused reading and export",
            &args,
            "topic",
            &topic,
            |topic| {
                format!(
                    "Conduct a literature review on: {topic}.\n\
                     1. Call search_papers with query \"{topic}\" and max_results {max_papers}.\n\
                     2. Pick the strongest papers from the deduplicated results: prefer recent\n\
                        work, high citation counts, and papers from multiple sources.\n\
                     3. For each chosen paper: call get_paper with enrich enabled (use the tldr),\n\
                        then get_paper_outline, then read_paper_section for the sections that\n\
                        matter (abstract, results, discussion), carrying the resolved source into\n\
                        both calls. Read Methods only if the claim depends on methodology. Use\n\
                        max_chars and paginate with the returned section-relative byte offset\n\
                        next_start as offset when a section is truncated.\n\
                     4. Synthesize: state themes, agreements, contradictions, and open gaps.\n\
                        Cite every claim as (title, source, paper_id).\n\
                     5. Finish with export_papers in bibtex format covering the reviewed papers."
                )
            },
        )
    }
}

/// Deep single-paper analysis driven by outlines and bounded section reads.
#[derive(Debug, Default)]
pub struct DeepPaperAnalysisPrompt;

#[async_trait::async_trait]
impl PromptHandler for DeepPaperAnalysisPrompt {
    async fn handle(
        &self,
        args: HashMap<String, String>,
        _extra: RequestHandlerExtra,
    ) -> Result<GetPromptResult> {
        let paper_id = arg(&args, "paper_id", "");
        if paper_id.trim().is_empty() {
            return Err(Error::invalid_params("Missing 'paper_id' argument"));
        }
        prompt(
            "Analyze one paper in depth: claims, evidence, limitations, follow-up reading",
            &args,
            "paper_id",
            &paper_id,
            |paper_id| {
                format!(
                    "Analyze the paper '{paper_id}' in depth.\n\
                     1. get_paper with paper_id '{paper_id}' (enrich on): capture the tldr, DOI,\n\
                        license and citation count.\n\
                     2. Carry the resolved source into get_paper_outline and read_paper_section\n\
                        for abstract, results and discussion. Keep each read under max_chars\n\
                        12000; when truncated pass next_start as the section-relative byte offset.\n\
                     3. Report: main claims, the evidence behind each, stated limitations, and\n\
                        anything the abstract oversells.\n\
                     4. Call get_citations and get_references; list the three most important\n\
                        follow-up papers with one line each on why.\n\
                     5. If a DOI is present, run get_citation_tallies and report supporting vs\n\
                        contradicting counts; mention any retraction flags."
                )
            },
        )
    }
}

/// Discovery walk from a seed paper or topic through recommendations.
#[derive(Debug, Default)]
pub struct ResearchDiscoveryPrompt;

#[async_trait::async_trait]
impl PromptHandler for ResearchDiscoveryPrompt {
    async fn handle(
        &self,
        args: HashMap<String, String>,
        _extra: RequestHandlerExtra,
    ) -> Result<GetPromptResult> {
        let seed = arg(&args, "seed", "");
        if seed.trim().is_empty() {
            return Err(Error::invalid_params("Missing 'seed' argument"));
        }
        prompt(
            "Build an annotated reading list from a seed paper or topic",
            &args,
            "seed",
            &seed,
            |seed| {
                let seed_note: String = if seed.starts_with("10.") {
                    "The seed is a DOI: call get_paper with paper_id set to it.".to_string()
                } else {
                    format!(
                        "search_papers with query \"{seed}\" and max_results 5, then pick\nthe best match and note its paper_id."
                    )
                };
                format!(
                    "Build a reading list starting from the seed: {seed}.\n\
                     1. {seed_note}\n\
                     2. Call recommend_papers with that paper_id to get related work.\n\
                     3. For each of the five most promising recommendations, call get_paper\n\
                        with enrichment enabled and record tldr, citation count and license.\n\
                     4. Order the list by reading priority (recency, citations, open access)\n\
                        with one sentence per paper on what it adds over the seed.\n\
                     5. If this is an active topic, call watch_topic so new papers surface on\n\
                        later check_alerts calls."
                )
            },
        )
    }
}

/// Verify a paper's evidence base: smart-citation tallies and retraction flags.
#[derive(Debug, Default)]
pub struct CitationAuditPrompt;

#[async_trait::async_trait]
impl PromptHandler for CitationAuditPrompt {
    async fn handle(
        &self,
        args: HashMap<String, String>,
        _extra: RequestHandlerExtra,
    ) -> Result<GetPromptResult> {
        let paper_id = arg(&args, "paper_id", "");
        if paper_id.trim().is_empty() {
            return Err(Error::invalid_params("Missing 'paper_id' argument"));
        }
        prompt(
            "Audit a paper's evidence base: citations, tallies, retractions",
            &args,
            "paper_id",
            &paper_id,
            |paper_id| {
                format!(
                    "Audit the evidence base of '{paper_id}'.\n\
                     1. get_paper with paper_id '{paper_id}': confirm the DOI and record the\n\
                        retraction field if present.\n\
                     2. If a DOI is present, get_citation_tallies and report supporting,\n\
                        contradicting and mentioning counts. A high contradicting share is a\n\
                        warning. Without a DOI, report tallies unavailable and continue.\n\
                     3. get_citations for citing papers and get_references for its basis; sample\n\
                        three references and check each with get_paper for retraction flags.\n\
                     4. Verdict: cite this, cite with caveats, or avoid. Justify with the numbers\n\
                        and name any retracted references explicitly."
                )
            },
        )
    }
}

/// Batch title/abstract screening against review criteria with PRISMA counting.
#[derive(Debug, Default)]
pub struct SystematicReviewScreeningPrompt;

#[async_trait::async_trait]
impl PromptHandler for SystematicReviewScreeningPrompt {
    async fn handle(
        &self,
        args: HashMap<String, String>,
        _extra: RequestHandlerExtra,
    ) -> Result<GetPromptResult> {
        let topic = arg(&args, "topic", "");
        if topic.trim().is_empty() {
            return Err(Error::invalid_params("Missing 'topic' argument"));
        }
        let inclusion = arg(&args, "inclusion_criteria", "");
        if inclusion.trim().is_empty() {
            return Err(Error::invalid_params(
                "Missing 'inclusion_criteria' argument",
            ));
        }
        let exclusion = arg(&args, "exclusion_criteria", "");
        if exclusion.trim().is_empty() {
            return Err(Error::invalid_params(
                "Missing 'exclusion_criteria' argument",
            ));
        }
        prompt(
            "Screen papers against review criteria in batches with PRISMA flow counts",
            &args,
            "topic",
            &topic,
            |topic| {
                format!(
                    "Screen papers on '{topic}' for a systematic review.\n\
                     Inclusion criteria: {inclusion}\n\
                     Exclusion criteria: {exclusion}\n\
                     1. Call screening_start with these criteria. Pass query '{topic}' to\n\
                        search the sources, or papers if a candidate list already exists;\n\
                        note the session_id.\n\
                     2. Loop: screening_next with the session_id returns a batch. Judge each\n\
                        candidate against both criteria using title, abstract, type, venue\n\
                        and the extracted_statistics field.\n\
                     3. Record each batch with screening_decide: 'include' when the paper\n\
                        meets the inclusion criteria; otherwise 'exclude' with a short reason\n\
                        naming the criterion it fails (that reason becomes the PRISMA\n\
                        exclusion category). Use 'uncertain' only when the abstract is\n\
                        genuinely ambiguous.\n\
                     4. When screening_next reports nothing remaining, call screening_summary\n\
                        and present the PRISMA flow: identified, duplicates removed, screened,\n\
                        excluded by reason, included, uncertain."
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn literature_review_requires_topic() {
        let result = LiteratureReviewPrompt
            .handle(HashMap::new(), test_extra())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn literature_review_interpolates_topic() {
        let mut args = HashMap::new();
        args.insert("topic".to_string(), "graph neural networks".to_string());
        let result = LiteratureReviewPrompt
            .handle(args, test_extra())
            .await
            .unwrap();
        let text = match &result.messages[0].content {
            Content::Text { text } => text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("graph neural networks"));
        assert!(text.contains("search_papers"));
    }

    #[tokio::test]
    async fn citation_audit_mentions_tallies() {
        let mut args = HashMap::new();
        args.insert("paper_id".to_string(), "10.1000/example".to_string());
        let result = CitationAuditPrompt
            .handle(args, test_extra())
            .await
            .unwrap();
        let text = match &result.messages[0].content {
            Content::Text { text } => text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("get_citation_tallies"));
        assert!(text.contains("retraction"));
    }

    #[tokio::test]
    async fn systematic_review_screening_requires_topic() {
        let result = SystematicReviewScreeningPrompt
            .handle(HashMap::new(), test_extra())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn systematic_review_screening_mentions_tool_flow() {
        let mut args = HashMap::new();
        args.insert("topic".to_string(), "digital therapeutics".to_string());
        args.insert(
            "inclusion_criteria".to_string(),
            "RCTs in adults".to_string(),
        );
        args.insert(
            "exclusion_criteria".to_string(),
            "animal studies".to_string(),
        );
        let result = SystematicReviewScreeningPrompt
            .handle(args, test_extra())
            .await
            .unwrap();
        let text = match &result.messages[0].content {
            Content::Text { text } => text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("digital therapeutics"));
        assert!(text.contains("RCTs in adults"));
        assert!(text.contains("screening_start"));
        assert!(text.contains("screening_next"));
        assert!(text.contains("screening_decide"));
        assert!(text.contains("screening_summary"));
    }

    fn test_extra() -> RequestHandlerExtra {
        RequestHandlerExtra::new(
            "test-request".to_string(),
            tokio_util::sync::CancellationToken::new(),
        )
    }
}
