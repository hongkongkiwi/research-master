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
    let year = extract_year(paper.published_date.as_deref());
    let title = &paper.title;
    let source = paper.source.name();
    let doi = paper.doi.as_deref().unwrap_or("");

    if !doi.is_empty() {
        format!(
            "{}. ({}). {}. {}. https://doi.org/{}",
            authors, year, title, source, doi
        )
    } else {
        format!("{}. ({}). {}. {}.", authors, year, title, source)
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
    let year = extract_year(paper.published_date.as_deref());
    let title = &paper.title;
    let source = paper.source.name();
    let doi = paper.doi.as_deref().unwrap_or("");

    if !doi.is_empty() {
        format!(
            "{}. {}. \"{}\". {}. https://doi.org/{}.",
            authors, year, title, source, doi
        )
    } else {
        format!("{}. {}. \"{}\". {}.", authors, year, title, source)
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
    let bibtex_authors: String = if authors.contains(';') {
        let author_list: Vec<&str> = authors.split(';').map(|s| s.trim()).collect();
        author_list
            .iter()
            .map(|a| {
                let parts: Vec<&str> = a.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 2 {
                    format!(
                        "{} and {} {}",
                        parts[0].trim(),
                        parts[1].trim(),
                        parts[0].trim()
                    )
                } else {
                    // Try "First Last" format
                    let words: Vec<&str> = a.split_whitespace().collect();
                    if words.len() >= 2 {
                        format!(
                            "{} and {} {}",
                            words.last().expect("already checked: words.len() >= 2"),
                            words[..words.len() - 1].join(" "),
                            words.last().expect("already checked")
                        )
                    } else {
                        a.to_string()
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" and ")
    } else {
        let parts: Vec<&str> = authors.split(',').map(|s| s.trim()).collect();
        if parts.len() >= 2 {
            format!(
                "{} and {} {}",
                parts[0].trim(),
                parts[1].trim(),
                parts[0].trim()
            )
        } else {
            authors.to_string()
        }
    };

    let year = extract_year(paper.published_date.as_deref());

    format!("@article{{{},\n  author = {{{}}},\n  title = {{{}}},\n  journal = {{{}}},\n  year = {{{}}},\n  url = {{{}}}\n}}",
        key, bibtex_authors, paper.title, paper.source.name(), year, paper.url)
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
