//! Citation formatting in various styles.
//!
//! Supports APA 7th, MLA 9th, Chicago 17th, and BibTeX formats.

use crate::models::Paper;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Citation style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CitationStyle {
    /// APA 7th edition
    Apa,
    /// MLA 9th edition
    Mla,
    /// Chicago 17th edition (author-date)
    Chicago,
    /// BibTeX
    Bibtex,
}

/// Format a paper citation in the specified style
pub fn format_citation(paper: &Paper, style: CitationStyle) -> String {
    match style {
        CitationStyle::Apa => format_apa(paper),
        CitationStyle::Mla => format_mla(paper),
        CitationStyle::Chicago => format_chicago(paper),
        CitationStyle::Bibtex => format_bibtex(paper),
    }
}

/// Format authors as "Last, F. M., & Last, F. M."
fn format_authors_apa(authors: &str) -> String {
    if authors.trim().is_empty() {
        return "Anonymous".to_string();
    }

    let author_list: Vec<&str> = authors.split(';').map(|s| s.trim()).collect();

    if author_list.len() == 1 {
        format_author_apa_single(author_list[0])
    } else if author_list.len() == 2 {
        format!(
            "{} & {}",
            format_author_apa_single(author_list[0]),
            format_author_apa_single(author_list[1])
        )
    } else if author_list.len() <= 20 {
        let formatted: Vec<String> = author_list
            .iter()
            .map(|a| format_author_apa_single(a))
            .collect();
        let all_but_last = formatted[..formatted.len() - 1].join(", ");
        format!(
            "{} & {}",
            all_but_last,
            formatted
                .last()
                .expect("already checked: format_apa has >= 2 authors")
        )
    } else {
        // APA: up to 20 authors, then ellipsis
        let formatted: Vec<String> = author_list[..20]
            .iter()
            .map(|a| format_author_apa_single(a))
            .collect();
        let all_but_last = formatted[..formatted.len() - 1].join(", ");
        format!(
            "{} ... {}",
            all_but_last,
            formatted
                .last()
                .expect("already checked: >20 authors, at least 20")
        )
    }
}

fn format_author_apa_single(author: &str) -> String {
    let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        // Already in "Last, First" format
        let last = parts[0].trim();
        let first = parts[1].trim();
        let initials: String = first
            .split_whitespace()
            .filter_map(|n| n.chars().next())
            .collect();
        format!("{}, {}.", last, initials)
    } else {
        // Try "First Last" format
        let words: Vec<&str> = author.split_whitespace().collect();
        if words.len() >= 2 {
            let last = words.last().expect("already checked: words.len() >= 2");
            let initials: String = words[..words.len() - 1]
                .iter()
                .filter_map(|n| n.chars().next())
                .collect();
            format!("{}, {}.", last, initials)
        } else {
            author.to_string()
        }
    }
}

/// Format authors as "Last, First, and First Last"
fn format_authors_mla(authors: &str) -> String {
    if authors.trim().is_empty() {
        return "Anonymous".to_string();
    }

    let author_list: Vec<&str> = authors.split(';').map(|s| s.trim()).collect();

    if author_list.len() == 1 {
        format_author_mla_single(author_list[0])
    } else if author_list.len() == 2 {
        format!(
            "{} and {}",
            format_author_mla_single(author_list[0]),
            format_author_mla_remaining(author_list[1])
        )
    } else {
        format!("{} et al", format_author_mla_single(author_list[0]))
    }
}

fn format_author_mla_single(author: &str) -> String {
    let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        // "Last, First"
        format!("{}, {}", parts[0].trim(), parts[1].trim())
    } else {
        // "First Last"
        let words: Vec<&str> = author.split_whitespace().collect();
        if words.len() >= 2 {
            format!(
                "{}, {}",
                words.last().expect("already checked: words.len() >= 2"),
                words[..words.len() - 1].join(" ")
            )
        } else {
            author.to_string()
        }
    }
}

fn format_author_mla_remaining(author: &str) -> String {
    let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[1].trim(), parts[0].trim())
    } else {
        author.to_string()
    }
}

/// Format authors as "Last, First"
fn format_authors_chicago(authors: &str) -> String {
    if authors.trim().is_empty() {
        return "Anonymous".to_string();
    }

    let author_list: Vec<&str> = authors.split(';').map(|s| s.trim()).collect();

    if author_list.len() == 1 {
        format_author_chicago_single(author_list[0])
    } else if author_list.len() == 2 {
        format!(
            "{} and {}",
            format_author_chicago_single(author_list[0]),
            format_author_chicago_remaining(author_list[1])
        )
    } else {
        format!("{} et al.", format_author_chicago_single(author_list[0]))
    }
}

fn format_author_chicago_single(author: &str) -> String {
    let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        format!("{}, {}", parts[0].trim(), parts[1].trim())
    } else {
        let words: Vec<&str> = author.split_whitespace().collect();
        if words.len() >= 2 {
            format!(
                "{}, {}",
                words.last().expect("already checked: words.len() >= 2"),
                words[..words.len() - 1].join(" ")
            )
        } else {
            author.to_string()
        }
    }
}

fn format_author_chicago_remaining(author: &str) -> String {
    let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[1].trim(), parts[0].trim())
    } else {
        author.to_string()
    }
}

/// Extract year from published_date (YYYY-MM-DD or YYYY)
fn extract_year(date: Option<&str>) -> String {
    match date {
        Some(d) => {
            if d.len() >= 4 {
                d[..4].to_string()
            } else {
                "n.d.".to_string()
            }
        }
        None => "n.d.".to_string(),
    }
}

/// Format paper in APA 7th edition
/// Format: Author, A. A., & Author, B. B. (Year). Title. Source. DOI
fn format_apa(paper: &Paper) -> String {
    let authors = format_authors_apa(&paper.authors);
    let author_sentence = if authors.ends_with('.') {
        authors
    } else {
        format!("{}.", authors)
    };
    let year = extract_year(paper.published_date.as_deref());
    let title = &paper.title;
    let source = paper.source.name();
    let doi = paper.doi.as_deref().unwrap_or("");

    if !doi.is_empty() {
        format!(
            "{} ({}). {}. {}. https://doi.org/{}",
            author_sentence, year, title, source, doi
        )
    } else {
        format!("{} ({}). {}. {}.", author_sentence, year, title, source)
    }
}

/// Format paper in MLA 9th edition
/// Format: Author. "Title." Source, Year, DOI.
fn format_mla(paper: &Paper) -> String {
    let authors = format_authors_mla(&paper.authors);
    let year = extract_year(paper.published_date.as_deref());
    let title = &paper.title;
    let formatted_title = format!("\"{}\"", title);
    let source = paper.source.name();
    let doi = paper.doi.as_deref().unwrap_or("");

    if !doi.is_empty() {
        format!(
            "{}. {}. {}, {}. https://doi.org/{}.",
            authors, formatted_title, source, year, doi
        )
    } else {
        format!("{}. {}. {}, {}.", authors, formatted_title, source, year)
    }
}

/// Format paper in Chicago 17th edition (author-date)
/// Format: Author. Year. "Title." Source. DOI.
fn format_chicago(paper: &Paper) -> String {
    let authors = format_authors_chicago(&paper.authors);
    let author_sentence = if authors.ends_with('.') {
        authors
    } else {
        format!("{}.", authors)
    };
    let year = extract_year(paper.published_date.as_deref());
    let title = &paper.title;
    let source = paper.source.name();
    let doi = paper.doi.as_deref().unwrap_or("");

    if !doi.is_empty() {
        format!(
            "{} {}. \"{}\". {}. https://doi.org/{}.",
            author_sentence, year, title, source, doi
        )
    } else {
        format!("{} {}. \"{}\". {}.", author_sentence, year, title, source)
    }
}

/// Generate a BibTeX entry
/// Format: @article{key,
///   author = {Last, First and Last, First},
///   title = {Title},
///   journal = {Source},
///   year = {Year},
///   doi = {DOI}
/// }
fn format_bibtex(paper: &Paper) -> String {
    // Generate citation key: FirstAuthorLastYearPaperTitle
    let authors = &paper.authors;
    let first_author = authors.split(';').next().unwrap_or("unknown").trim();
    let last_name = first_author
        .split(',')
        .next()
        .unwrap_or(first_author)
        .trim();
    let last_name = last_name.split_whitespace().last().unwrap_or(last_name);
    let year = extract_year(paper.published_date.as_deref());
    let title_words: Vec<&str> = paper.title.split_whitespace().take(3).collect();
    let title_key: String = title_words
        .iter()
        .map(|w| {
            let cleaned: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            cleaned
        })
        .collect();

    let key = format!("{}{}{}", last_name, year, title_key);

    // Format authors for BibTeX (Last, First and Last, First)
    let bibtex_authors: String = authors
        .split(';')
        .map(|a| {
            let author = a.trim();
            let parts: Vec<&str> = author.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 2 {
                format!("{}, {}", parts[0].trim(), parts[1].trim())
            } else {
                // Try "First Last" format
                let words: Vec<&str> = author.split_whitespace().collect();
                if words.len() >= 2 {
                    format!(
                        "{}, {}",
                        words.last().expect("already checked: words.len() >= 2"),
                        words[..words.len() - 1].join(" ")
                    )
                } else {
                    author.to_string()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" and ");

    let year = extract_year(paper.published_date.as_deref());

    let doi_field = paper
        .doi
        .as_deref()
        .map(|doi| format!("\n  doi = {{{}}},", doi))
        .unwrap_or_default();

    format!(
        "@article{{{},\n  author = {{{}}},\n  title = {{{}}},\n  journal = {{{}}},\n  year = {{{}}},{}\n  url = {{{}}}\n}}",
        key,
        bibtex_authors,
        paper.title,
        paper.source.name(),
        year,
        doi_field,
        paper.url
    )
}

/// Structured citation data for JSON output
#[derive(Debug, Serialize)]
pub struct StructuredCitation {
    pub style: String,
    pub formatted: String,
    pub authors: String,
    pub title: String,
    pub year: String,
    pub source: String,
    pub doi: Option<String>,
    pub url: String,
}

/// Get structured citation data
pub fn get_structured_citation(paper: &Paper, style: CitationStyle) -> StructuredCitation {
    StructuredCitation {
        style: format!("{:?}", style),
        formatted: format_citation(paper, style),
        authors: paper.authors.clone(),
        title: paper.title.clone(),
        year: extract_year(paper.published_date.as_deref()).to_string(),
        source: paper.source.name().to_string(),
        doi: paper.doi.clone(),
        url: paper.url.clone(),
    }
}

impl fmt::Display for CitationStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CitationStyle::Apa => write!(f, "APA 7th"),
            CitationStyle::Mla => write!(f, "MLA 9th"),
            CitationStyle::Chicago => write!(f, "Chicago 17th"),
            CitationStyle::Bibtex => write!(f, "BibTeX"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Paper, PaperBuilder, SourceType};

    const PAPER_ID: &str = "paper-1";
    const TITLE: &str = "Citation Testing in Rust";
    const URL: &str = "https://example.com/paper";
    const DOI: &str = "10.1234/example";

    fn paper(authors: &str, doi: Option<&str>) -> Paper {
        let builder = PaperBuilder::new(PAPER_ID, TITLE, URL, SourceType::Arxiv)
            .authors(authors)
            .published_date("2024-03-15");

        match doi {
            Some(doi) => builder.doi(doi).build(),
            None => builder.build(),
        }
    }

    fn paper_without_date(authors: &str) -> Paper {
        PaperBuilder::new(PAPER_ID, TITLE, URL, SourceType::Arxiv)
            .authors(authors)
            .build()
    }

    #[test]
    fn test_citation_style_display() {
        assert_eq!(CitationStyle::Apa.to_string(), "APA 7th");
        assert_eq!(CitationStyle::Mla.to_string(), "MLA 9th");
        assert_eq!(CitationStyle::Chicago.to_string(), "Chicago 17th");
        assert_eq!(CitationStyle::Bibtex.to_string(), "BibTeX");
    }

    #[test]
    fn test_apa_single_author_without_doi() {
        let citation = format_citation(&paper("Doe, John", None), CitationStyle::Apa);

        assert_eq!(citation, "Doe, J. (2024). Citation Testing in Rust. arXiv.");
    }

    #[test]
    fn test_apa_two_authors_without_doi() {
        let citation = format_citation(&paper("Doe, John; Smith, Jane", None), CitationStyle::Apa);

        assert_eq!(
            citation,
            "Doe, J. & Smith, J. (2024). Citation Testing in Rust. arXiv."
        );
    }

    #[test]
    fn test_apa_many_authors_without_doi() {
        let citation = format_citation(
            &paper("Doe, John; Smith, Jane; Brown, Bob", None),
            CitationStyle::Apa,
        );

        assert_eq!(
            citation,
            "Doe, J., Smith, J. & Brown, B. (2024). Citation Testing in Rust. arXiv."
        );
    }

    #[test]
    fn test_apa_no_authors_uses_anonymous() {
        let citation = format_citation(&paper("", None), CitationStyle::Apa);

        assert_eq!(
            citation,
            "Anonymous. (2024). Citation Testing in Rust. arXiv."
        );
    }

    #[test]
    fn test_apa_with_doi() {
        let citation = format_citation(&paper("Doe, John", Some(DOI)), CitationStyle::Apa);

        assert_eq!(
            citation,
            "Doe, J. (2024). Citation Testing in Rust. arXiv. https://doi.org/10.1234/example"
        );
    }

    #[test]
    fn test_mla_single_author() {
        let citation = format_citation(&paper("Doe, John", None), CitationStyle::Mla);

        assert_eq!(
            citation,
            "Doe, John. \"Citation Testing in Rust\". arXiv, 2024."
        );
    }

    #[test]
    fn test_mla_two_authors() {
        let citation = format_citation(&paper("Doe, John; Smith, Jane", None), CitationStyle::Mla);

        assert_eq!(
            citation,
            "Doe, John and Jane Smith. \"Citation Testing in Rust\". arXiv, 2024."
        );
    }

    #[test]
    fn test_mla_three_or_more_authors_uses_et_al() {
        let citation = format_citation(
            &paper("Doe, John; Smith, Jane; Brown, Bob", None),
            CitationStyle::Mla,
        );

        assert_eq!(
            citation,
            "Doe, John et al. \"Citation Testing in Rust\". arXiv, 2024."
        );
    }

    #[test]
    fn test_chicago_single_author() {
        let citation = format_citation(&paper("Doe, John", None), CitationStyle::Chicago);

        assert_eq!(
            citation,
            "Doe, John. 2024. \"Citation Testing in Rust\". arXiv."
        );
    }

    #[test]
    fn test_chicago_two_authors() {
        let citation = format_citation(
            &paper("Doe, John; Smith, Jane", None),
            CitationStyle::Chicago,
        );

        assert_eq!(
            citation,
            "Doe, John and Jane Smith. 2024. \"Citation Testing in Rust\". arXiv."
        );
    }

    #[test]
    fn test_chicago_three_or_more_authors_uses_et_al() {
        let citation = format_citation(
            &paper("Doe, John; Smith, Jane; Brown, Bob", None),
            CitationStyle::Chicago,
        );

        assert_eq!(
            citation,
            "Doe, John et al. 2024. \"Citation Testing in Rust\". arXiv."
        );
    }

    #[test]
    fn test_bibtex_author_formats_key_generation_and_doi() {
        let citation = format_citation(
            &paper("Doe, John; Jane Smith", Some(DOI)),
            CitationStyle::Bibtex,
        );

        assert_eq!(
            citation,
            "@article{Doe2024CitationTestingin,\n  author = {Doe, John and Smith, Jane},\n  title = {Citation Testing in Rust},\n  journal = {arXiv},\n  year = {2024},\n  doi = {10.1234/example},\n  url = {https://example.com/paper}\n}"
        );
    }

    #[test]
    fn test_bibtex_without_doi() {
        let citation = format_citation(&paper("John Michael Doe", None), CitationStyle::Bibtex);

        assert_eq!(
            citation,
            "@article{Doe2024CitationTestingin,\n  author = {Doe, John Michael},\n  title = {Citation Testing in Rust},\n  journal = {arXiv},\n  year = {2024},\n  url = {https://example.com/paper}\n}"
        );
        assert!(!citation.contains("doi ="));
    }

    #[test]
    fn test_extract_year() {
        assert_eq!(extract_year(Some("2024-03-15")), "2024");
        assert_eq!(extract_year(Some("2024")), "2024");
        assert_eq!(extract_year(Some("")), "n.d.");
        assert_eq!(extract_year(None), "n.d.");
    }

    #[test]
    fn test_get_structured_citation_basic_structure() {
        let source_paper = paper("Doe, John", Some(DOI));
        let structured = get_structured_citation(&source_paper, CitationStyle::Mla);

        assert_eq!(structured.style, "Mla");
        assert_eq!(
            structured.formatted,
            format_citation(&source_paper, CitationStyle::Mla)
        );
        assert_eq!(structured.authors, "Doe, John");
        assert_eq!(structured.title, TITLE);
        assert_eq!(structured.year, "2024");
        assert_eq!(structured.source, "arXiv");
        assert_eq!(structured.doi, Some(DOI.to_string()));
        assert_eq!(structured.url, URL);
    }

    #[test]
    fn test_get_structured_citation_without_date_uses_no_date() {
        let source_paper = paper_without_date("Doe, John");
        let structured = get_structured_citation(&source_paper, CitationStyle::Apa);

        assert_eq!(structured.year, "n.d.");
        assert!(structured.formatted.contains("(n.d.)"));
    }

    #[test]
    fn test_format_author_apa_single() {
        assert_eq!(format_author_apa_single("Doe, John Michael"), "Doe, JM.");
        assert_eq!(format_author_apa_single("John Michael Doe"), "Doe, JM.");
        assert_eq!(format_author_apa_single("Plato"), "Plato");
    }

    #[test]
    fn test_format_author_mla_single() {
        assert_eq!(
            format_author_mla_single("Doe, John Michael"),
            "Doe, John Michael"
        );
        assert_eq!(
            format_author_mla_single("John Michael Doe"),
            "Doe, John Michael"
        );
    }
}
