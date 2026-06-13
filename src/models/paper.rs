//! Paper model representing a research paper from any source.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The source/repository where the paper was found
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Arxiv,
    PubMed,
    BioRxiv,
    MedRxiv,
    SemanticScholar,
    OpenAlex,
    OpenAIRE,
    CrossRef,
    IACR,
    PMC,
    HAL,
    DBLP,
    SSRN,
    GoogleScholar,
    SciHub,
    CORE,
    CiteSeerX,
    EuropePMC,
    Dimensions,
    IeeeXplore,
    Zenodo,
    Unpaywall,
    MDPI,
    Jstor,
    Scispace,
    Acm,
    ConnectedPapers,
    Doaj,
    WorldWideScience,
    Osf,
    Base,
    Springer,
    #[serde(untagged)]
    Other(String),
}

impl SourceType {
    /// Returns the display name of the source
    pub fn name(&self) -> &str {
        match self {
            SourceType::Arxiv => "arXiv",
            SourceType::PubMed => "PubMed",
            SourceType::BioRxiv => "bioRxiv",
            SourceType::MedRxiv => "medRxiv",
            SourceType::SemanticScholar => "Semantic Scholar",
            SourceType::OpenAlex => "OpenAlex",
            SourceType::OpenAIRE => "OpenAIRE",
            SourceType::CrossRef => "CrossRef",
            SourceType::IACR => "IACR ePrint",
            SourceType::PMC => "PubMed Central",
            SourceType::HAL => "HAL",
            SourceType::DBLP => "DBLP",
            SourceType::SSRN => "SSRN",
            SourceType::GoogleScholar => "Google Scholar",
            SourceType::SciHub => "Sci-Hub",
            SourceType::CORE => "CORE",
            SourceType::CiteSeerX => "CiteSeerX",
            SourceType::EuropePMC => "Europe PMC",
            SourceType::Dimensions => "Dimensions",
            SourceType::IeeeXplore => "IEEE Xplore",
            SourceType::Zenodo => "Zenodo",
            SourceType::Unpaywall => "Unpaywall",
            SourceType::MDPI => "MDPI",
            SourceType::Jstor => "JSTOR",
            SourceType::Scispace => "SciSpace",
            SourceType::Acm => "ACM Digital Library",
            SourceType::ConnectedPapers => "Connected Papers",
            SourceType::Doaj => "DOAJ",
            SourceType::WorldWideScience => "WorldWideScience",
            SourceType::Osf => "OSF Preprints",
            SourceType::Base => "BASE",
            SourceType::Springer => "Springer",
            SourceType::Other(s) => s,
        }
    }

    /// Returns the source identifier (for tool naming)
    pub fn id(&self) -> &str {
        match self {
            SourceType::Arxiv => "arxiv",
            SourceType::PubMed => "pubmed",
            SourceType::BioRxiv => "biorxiv",
            SourceType::MedRxiv => "medrxiv",
            SourceType::SemanticScholar => "semantic",
            SourceType::OpenAlex => "openalex",
            SourceType::OpenAIRE => "openaire",
            SourceType::CrossRef => "crossref",
            SourceType::IACR => "iacr",
            SourceType::PMC => "pmc",
            SourceType::HAL => "hal",
            SourceType::DBLP => "dblp",
            SourceType::SSRN => "ssrn",
            SourceType::GoogleScholar => "google_scholar",
            SourceType::SciHub => "sci_hub",
            SourceType::CORE => "core",
            SourceType::CiteSeerX => "citeseerx",
            SourceType::EuropePMC => "europe_pmc",
            SourceType::Dimensions => "dimensions",
            SourceType::IeeeXplore => "ieee_xplore",
            SourceType::Zenodo => "zenodo",
            SourceType::Unpaywall => "unpaywall",
            SourceType::MDPI => "mdpi",
            SourceType::Jstor => "jstor",
            SourceType::Scispace => "scispace",
            SourceType::Acm => "acm",
            SourceType::ConnectedPapers => "connected_papers",
            SourceType::Doaj => "doaj",
            SourceType::WorldWideScience => "worldwidescience",
            SourceType::Osf => "osf",
            SourceType::Base => "base",
            SourceType::Springer => "springer",
            SourceType::Other(s) => s,
        }
    }
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A research paper from any academic source
///
/// This struct provides a standardized format for papers across all sources,
/// making it easy to work with papers from multiple repositories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    /// Unique identifier (source-specific: DOI, PMID, arXiv ID, etc.)
    pub paper_id: String,

    /// Paper title
    pub title: String,

    /// Authors (semicolon-separated)
    pub authors: String,

    /// Abstract text
    pub r#abstract: String,

    /// Digital Object Identifier
    pub doi: Option<String>,

    /// Publication date (ISO format)
    pub published_date: Option<String>,

    /// Last updated date (ISO format)
    pub updated_date: Option<String>,

    /// Direct PDF URL
    pub pdf_url: Option<String>,

    /// Paper page URL
    pub url: String,

    /// Source where the paper was found
    pub source: SourceType,

    /// Categories/tags (semicolon-separated)
    pub categories: Option<String>,

    /// Keywords (semicolon-separated)
    pub keywords: Option<String>,

    /// Citation count
    pub citations: Option<u32>,

    /// Reference IDs (semicolon-separated)
    pub references: Option<String>,

    /// Source-specific metadata (flexible JSON)
    pub extra: Option<HashMap<String, serde_json::Value>>,
}

impl Paper {
    /// Create a new paper with required fields
    pub fn new(paper_id: String, title: String, url: String, source: SourceType) -> Self {
        Self {
            paper_id,
            title,
            authors: String::new(),
            r#abstract: String::new(),
            doi: None,
            published_date: None,
            updated_date: None,
            pdf_url: None,
            url,
            source,
            categories: None,
            keywords: None,
            citations: None,
            references: None,
            extra: None,
        }
    }

    /// Returns the primary identifier for this paper (DOI if available, else paper_id)
    pub fn primary_id(&self) -> &str {
        self.doi.as_ref().unwrap_or(&self.paper_id)
    }

    /// Returns the author names as a vector
    pub fn author_list(&self) -> Vec<&str> {
        self.authors
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Returns the categories as a vector
    pub fn category_list(&self) -> Vec<&str> {
        self.categories
            .as_ref()
            .map(|c| {
                c.split(';')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the keywords as a vector
    pub fn keyword_list(&self) -> Vec<&str> {
        self.keywords
            .as_ref()
            .map(|k| {
                k.split(';')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Check if paper has a downloadable PDF
    pub fn has_pdf(&self) -> bool {
        self.pdf_url.is_some()
    }
}

/// Builder for constructing Paper objects
#[derive(Debug, Clone)]
pub struct PaperBuilder {
    paper: Paper,
}

impl PaperBuilder {
    /// Create a new builder with required fields
    pub fn new(
        paper_id: impl Into<String>,
        title: impl Into<String>,
        url: impl Into<String>,
        source: SourceType,
    ) -> Self {
        Self {
            paper: Paper::new(paper_id.into(), title.into(), url.into(), source),
        }
    }

    /// Set authors
    pub fn authors(mut self, authors: impl Into<String>) -> Self {
        self.paper.authors = authors.into();
        self
    }

    /// Set abstract
    pub fn abstract_text(mut self, abstract_text: impl Into<String>) -> Self {
        self.paper.r#abstract = abstract_text.into();
        self
    }

    /// Set DOI
    pub fn doi(mut self, doi: impl Into<String>) -> Self {
        self.paper.doi = Some(doi.into());
        self
    }

    /// Set publication date
    pub fn published_date(mut self, date: impl Into<String>) -> Self {
        self.paper.published_date = Some(date.into());
        self
    }

    /// Set updated date
    pub fn updated_date(mut self, date: impl Into<String>) -> Self {
        self.paper.updated_date = Some(date.into());
        self
    }

    /// Set PDF URL
    pub fn pdf_url(mut self, url: impl Into<String>) -> Self {
        self.paper.pdf_url = Some(url.into());
        self
    }

    /// Set categories
    pub fn categories(mut self, categories: impl Into<String>) -> Self {
        self.paper.categories = Some(categories.into());
        self
    }

    /// Set keywords
    pub fn keywords(mut self, keywords: impl Into<String>) -> Self {
        self.paper.keywords = Some(keywords.into());
        self
    }

    /// Set citation count
    pub fn citations(mut self, count: u32) -> Self {
        self.paper.citations = Some(count);
        self
    }

    /// Set references
    pub fn references(mut self, references: impl Into<String>) -> Self {
        self.paper.references = Some(references.into());
        self
    }

    /// Add extra metadata
    pub fn extra(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.paper
            .extra
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value);
        self
    }

    /// Build the Paper
    pub fn build(self) -> Paper {
        self.paper
    }
}

impl std::fmt::Display for Paper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let year = self
            .published_date
            .as_ref()
            .map(|d| d.chars().take(4).collect::<String>())
            .unwrap_or_else(|| "????".to_string());

        // Truncate title if too long (keep 50 chars)
        let title_display = if self.title.len() > 50 {
            format!("{}...", &self.title[..47])
        } else {
            self.title.clone()
        };

        // Truncate authors if too long (keep 40 chars)
        let authors_display = if self.authors.len() > 40 {
            format!("{}...", &self.authors[..37])
        } else {
            self.authors.clone()
        };

        write!(
            f,
            "{} | {} | {} | {}",
            title_display, authors_display, self.source, year
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paper_builder() {
        let paper = PaperBuilder::new(
            "1234.5678",
            "Test Paper",
            "https://example.com",
            SourceType::Arxiv,
        )
        .authors("John Doe; Jane Smith")
        .abstract_text("This is a test abstract.")
        .doi("10.1234/test.1234")
        .pdf_url("https://example.com/paper.pdf")
        .citations(42)
        .build();

        assert_eq!(paper.paper_id, "1234.5678");
        assert_eq!(paper.title, "Test Paper");
        assert_eq!(paper.authors, "John Doe; Jane Smith");
        assert_eq!(paper.doi, Some("10.1234/test.1234".to_string()));
        assert_eq!(paper.citations, Some(42));
    }

    #[test]
    fn test_author_list() {
        let paper = PaperBuilder::new(
            "1234".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
            SourceType::Arxiv,
        )
        .authors("John Doe; Jane Smith; Bob Jones")
        .build();

        let authors = paper.author_list();
        assert_eq!(authors, vec!["John Doe", "Jane Smith", "Bob Jones"]);
    }

    #[test]
    fn test_primary_id() {
        let with_doi = PaperBuilder::new(
            "1234".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
            SourceType::Arxiv,
        )
        .doi("10.1234/test")
        .build();

        assert_eq!(with_doi.primary_id(), "10.1234/test");

        let without_doi = Paper::new(
            "1234".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
            SourceType::Arxiv,
        );

        assert_eq!(without_doi.primary_id(), "1234");
    }

    #[test]
    fn test_paper_builder_all_fields() {
        let paper = PaperBuilder::new(
            "PMC12345",
            "Medical Research Paper",
            "https://pubmed.ncbi.nlm.nih.gov/12345/",
            SourceType::PubMed,
        )
        .authors("Alice Johnson; Bob Williams")
        .abstract_text("This is a medical abstract.")
        .doi("10.1000/abc123")
        .pdf_url("https://example.com/fulltext.pdf")
        .published_date("2023-05-15")
        .categories("Medicine;Biology")
        .keywords("gene therapy;CRISPR")
        .citations(100)
        .references("ref1;ref2")
        .build();

        assert_eq!(paper.paper_id, "PMC12345");
        assert_eq!(paper.title, "Medical Research Paper");
        assert_eq!(paper.source, SourceType::PubMed);
        assert_eq!(paper.authors, "Alice Johnson; Bob Williams");
        assert_eq!(paper.doi, Some("10.1000/abc123".to_string()));
        assert_eq!(paper.published_date, Some("2023-05-15".to_string()));
        assert_eq!(paper.categories, Some("Medicine;Biology".to_string()));
        assert_eq!(paper.keywords, Some("gene therapy;CRISPR".to_string()));
        assert_eq!(paper.citations, Some(100));
        assert_eq!(paper.references, Some("ref1;ref2".to_string()));
    }

    #[test]
    fn test_paper_builder_empty_authors() {
        let paper = PaperBuilder::new(
            "1234",
            "Anonymous Paper",
            "https://example.com",
            SourceType::Arxiv,
        )
        .authors("")
        .build();

        let authors = paper.author_list();
        assert!(authors.is_empty());
    }

    #[test]
    fn test_paper_builder_minimal() {
        let paper = PaperBuilder::new(
            "minimal",
            "Minimal Paper",
            "https://example.com",
            SourceType::SemanticScholar,
        )
        .build();

        assert_eq!(paper.paper_id, "minimal");
        assert_eq!(paper.title, "Minimal Paper");
        assert!(paper.authors.is_empty());
        assert!(paper.doi.is_none());
        assert!(paper.r#abstract.is_empty());
    }

    #[test]
    fn test_paper_with_pdf() {
        let paper = PaperBuilder::new(
            "1234",
            "Paper with PDF",
            "https://example.com",
            SourceType::Arxiv,
        )
        .pdf_url("https://arxiv.org/pdf/1234.pdf")
        .build();

        assert!(paper.has_pdf());
        assert!(paper.pdf_url.is_some());
    }

    #[test]
    fn test_paper_without_pdf() {
        let paper = Paper::new(
            "1234".to_string(),
            "Paper without PDF".to_string(),
            "https://example.com".to_string(),
            SourceType::Arxiv,
        );

        assert!(!paper.has_pdf());
    }

    #[test]
    fn test_category_list() {
        let paper = PaperBuilder::new("1234", "Test", "https://example.com", SourceType::Arxiv)
            .categories("cs.AI;cs.LG")
            .build();

        let categories = paper.category_list();
        assert_eq!(categories, vec!["cs.AI", "cs.LG"]);
    }

    #[test]
    fn test_keyword_list() {
        let paper = PaperBuilder::new("1234", "Test", "https://example.com", SourceType::Arxiv)
            .keywords("neural networks;deep learning")
            .build();

        let keywords = paper.keyword_list();
        assert_eq!(keywords, vec!["neural networks", "deep learning"]);
    }
}
