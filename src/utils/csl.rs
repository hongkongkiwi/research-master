//! CSL style formatting backed by hayagriva's embedded style archive.
//!
//! Accepts any style name from the Citation Styles Locator (apa, ieee, nature,
//! vancouver, chicago-author-date, ...) instead of the four hand-rolled styles
//! in [`crate::utils::cite`].

use crate::models::Paper;
use hayagriva::archive::ArchivedStyle;
use hayagriva::{BibliographyDriver, BibliographyRequest, CitationItem, CitationRequest};

/// Format one paper as an in-text style citation in any archived CSL style.
///
/// The paper is serialized to BibLaTeX (reusing the existing bibtex writer),
/// parsed by hayagriva, and rendered with the named archived style.
pub fn format_csl_citation(paper: &Paper, style_name: &str) -> Result<String, String> {
    let bibtex = crate::utils::cite::format_citation(paper, crate::utils::CitationStyle::Bibtex);
    let library = hayagriva::io::from_biblatex_str(&bibtex)
        .map_err(|errors| format!("bibliography parse failed: {errors:?}"))?;
    let style = match ArchivedStyle::by_name(style_name) {
        Some(style) => style.get(),
        None => return Err(unknown_style_message(style_name)),
    };
    // Archived styles are all independent; dependent styles only redirect to
    // a parent style and cannot render by themselves.
    let style = match style {
        hayagriva::citationberg::Style::Independent(independent) => independent,
        hayagriva::citationberg::Style::Dependent(_) => {
            return Err(unknown_style_message(style_name));
        }
    };
    let locales = hayagriva::archive::locales();

    let mut driver = BibliographyDriver::new();
    for entry in library.iter() {
        let items = vec![CitationItem::with_entry(entry)];
        driver.citation(CitationRequest::from_items(items, &style, &locales));
    }
    let rendered = driver.finish(BibliographyRequest {
        style: &style,
        locale: None,
        locale_files: &locales,
    });
    Ok(rendered
        .citations
        .first()
        .map(|cite| cite.citation.to_string())
        .unwrap_or_default())
}

fn unknown_style_message(style_name: &str) -> String {
    let popular = ["apa", "ieee", "nature", "vancouver", "chicago-author-date"];
    format!(
        "unknown CSL style '{style_name}'. Use a style name from the CSL styles archive \
         (https://www.zotero.org/styles), for example: {}",
        popular.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_paper() -> Paper {
        let mut paper = Paper::new(
            "1706.03762".to_string(),
            "Attention Is All You Need".to_string(),
            "https://arxiv.org/abs/1706.03762".to_string(),
            crate::models::SourceType::Arxiv,
        );
        paper.authors = "Vaswani, Ashish; Shazeer, Noam".to_string();
        paper.published_date = Some("2017".to_string());
        paper.doi = Some("10.5555/3295222.3295349".to_string());
        paper
    }

    #[test]
    fn formats_apa_citation() {
        let text = format_csl_citation(&sample_paper(), "apa").unwrap();
        assert!(text.contains("Vaswani"), "unexpected: {text}");
        assert!(text.contains("2017"), "unexpected: {text}");
    }

    #[test]
    fn formats_ieee_citation() {
        // IEEE in-text citations are numeric: "[1]".
        let text = format_csl_citation(&sample_paper(), "ieee").unwrap();
        assert!(text.contains('['), "unexpected: {text}");
    }

    #[test]
    fn unknown_style_lists_suggestions() {
        let error = format_csl_citation(&sample_paper(), "not-a-style").unwrap_err();
        assert!(error.contains("unknown CSL style"));
        assert!(error.contains("ieee"));
    }
}
