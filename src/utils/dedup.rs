//! Deduplication utilities for papers across sources.

use std::collections::{HashMap, HashSet};
use strsim::jaro_winkler;

use crate::models::Paper;

/// Strategy for handling duplicates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateStrategy {
    /// Keep the first occurrence of each duplicate group
    First,
    /// Keep the last occurrence of each duplicate group
    Last,
    /// Keep all papers but mark duplicates
    Mark,
}

/// Find duplicate papers based on DOI, title similarity, and author+year
///
/// Returns groups of paper indices that are duplicates of each other
pub fn find_duplicates(papers: &[Paper]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut processed: HashSet<usize> = HashSet::new();

    for i in 0..papers.len() {
        if processed.contains(&i) {
            continue;
        }

        let mut group = vec![i];
        let paper_i = &papers[i];

        for (j, paper_j) in papers.iter().enumerate().skip(i + 1) {
            if processed.contains(&j) {
                continue;
            }

            // Check if papers are duplicates
            if are_duplicates(paper_i, paper_j) {
                group.push(j);
                processed.insert(j);
            }
        }

        if group.len() > 1 {
            groups.push(group);
        }

        processed.insert(i);
    }

    groups
}

/// Check if two papers are likely duplicates
fn are_duplicates(a: &Paper, b: &Paper) -> bool {
    // Same source means they're not duplicates
    if a.source == b.source {
        return false;
    }

    if let (Some(doi_a), Some(doi_b)) = (normalized_doi(a), normalized_doi(b)) {
        return doi_a == doi_b;
    }

    // Check title similarity
    let title_a = a.title.to_lowercase().trim().to_string();
    let title_b = b.title.to_lowercase().trim().to_string();

    let title_similarity = jaro_winkler(&title_a, &title_b);

    // High title similarity (0.95+ threshold)
    if title_similarity >= 0.95 {
        // Also check authors match approximately
        if authors_match(a, b) {
            return true;
        }
    }

    // Check exact title match after cleaning
    if normalize_title(&title_a) == normalize_title(&title_b) && authors_match(a, b) {
        return true;
    }

    false
}

fn normalized_doi(paper: &Paper) -> Option<String> {
    paper
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|doi| !doi.is_empty())
        .map(str::to_lowercase)
}

/// Check if authors approximately match
fn authors_match(a: &Paper, b: &Paper) -> bool {
    let authors_a: HashSet<String> = a
        .author_list()
        .iter()
        .map(|s| s.to_lowercase().trim().to_string())
        .collect();
    let authors_b: HashSet<String> = b
        .author_list()
        .iter()
        .map(|s| s.to_lowercase().trim().to_string())
        .collect();

    // If one has no authors, can't compare
    if authors_a.is_empty() || authors_b.is_empty() {
        return true; // Assume match if author info is missing
    }

    // Check if at least one author matches
    authors_a.intersection(&authors_b).count() > 0
}

/// Normalize a title for comparison
fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove duplicate papers from a list
///
/// # Arguments
/// * `papers` - The papers to deduplicate
/// * `strategy` - How to handle duplicates (keep first, last, or mark)
///
/// # Returns
/// A deduplicated list of papers
pub fn deduplicate_papers(papers: Vec<Paper>, strategy: DuplicateStrategy) -> Vec<Paper> {
    let groups = find_duplicates(&papers);

    if groups.is_empty() {
        return papers;
    }

    let mut to_remove: HashSet<usize> = HashSet::new();

    for group in groups {
        match strategy {
            DuplicateStrategy::First => {
                // Keep first, remove rest
                for idx in group.iter().skip(1) {
                    to_remove.insert(*idx);
                }
            }
            DuplicateStrategy::Last => {
                // Keep last, remove rest
                for idx in group.iter().take(group.len() - 1) {
                    to_remove.insert(*idx);
                }
            }
            DuplicateStrategy::Mark => {
                // Don't remove any, just return as-is
                // In a real implementation, you might add a "duplicate" field
            }
        }
    }

    if to_remove.is_empty() {
        return papers;
    }

    papers
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !to_remove.contains(i))
        .map(|(_, p)| p)
        .collect()
}

/// Fast hash-based deduplication for papers
///
/// Uses a two-pass algorithm for O(n) complexity on exact matches:
/// 1. First pass: Hash-based matching for DOIs and normalized titles (O(n))
/// 2. Second pass: Similarity check only for papers not matched by hash
///
/// This is significantly faster than the O(n²) similarity-only approach
/// for large paper lists with many exact matches.
pub fn fast_deduplicate_papers(papers: Vec<Paper>, strategy: DuplicateStrategy) -> Vec<Paper> {
    if papers.len() <= 1 {
        return papers;
    }

    // Maps for O(1) lookups
    let mut doi_map: HashMap<String, Vec<usize>> = HashMap::new();
    let mut title_map: HashMap<String, Vec<usize>> = HashMap::new();

    // First pass: build hash maps (O(n))
    for (idx, paper) in papers.iter().enumerate() {
        // Index by lowercase DOI
        if let Some(doi_key) = normalized_doi(paper) {
            doi_map.entry(doi_key).or_default().push(idx);
        }

        // Index by normalized title
        let normalized = normalize_title(&paper.title.to_lowercase());
        title_map.entry(normalized).or_default().push(idx);
    }

    // Track duplicates using a HashSet for O(1) lookups
    let mut duplicates: HashSet<usize> = HashSet::new();

    // Process DOI matches (strongest signal) - O(n)
    for (_, indices) in doi_map.into_iter() {
        if indices.len() > 1 {
            match strategy {
                DuplicateStrategy::First => {
                    for idx in indices.iter().skip(1) {
                        duplicates.insert(*idx);
                    }
                }
                DuplicateStrategy::Last => {
                    for idx in indices.iter().take(indices.len() - 1) {
                        duplicates.insert(*idx);
                    }
                }
                DuplicateStrategy::Mark => {
                    // Keep all
                }
            }
        }
    }

    // Process title matches for papers not already marked as DOI duplicates
    // Only compare papers from different sources to avoid false positives
    for (_, indices) in title_map.into_iter() {
        if indices.len() > 1 {
            let mut to_mark: Vec<usize> = Vec::new();

            // Check each pair
            for i in 0..indices.len() {
                if duplicates.contains(&indices[i]) {
                    continue;
                }

                for j in (i + 1)..indices.len() {
                    if duplicates.contains(&indices[j]) {
                        continue;
                    }

                    let paper_i = &papers[indices[i]];
                    let paper_j = &papers[indices[j]];

                    // Skip same source
                    if paper_i.source == paper_j.source {
                        continue;
                    }

                    // DOI matches are handled above; conflicting DOIs cannot match by title.
                    if let (Some(_), Some(_)) = (normalized_doi(paper_i), normalized_doi(paper_j)) {
                        continue;
                    }

                    // Additional similarity check for confidence
                    if title_similarity_confidence(paper_i, paper_j) {
                        match strategy {
                            DuplicateStrategy::First => to_mark.push(indices[j]),
                            DuplicateStrategy::Last => to_mark.push(indices[i]),
                            DuplicateStrategy::Mark => {}
                        }
                    }
                }
            }

            // Add to duplicates
            for idx in to_mark {
                duplicates.insert(idx);
            }
        }
    }

    // Filter papers
    papers
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !duplicates.contains(i))
        .map(|(_, p)| p)
        .collect()
}

/// Calculate confidence that two papers are the same based on multiple signals
fn title_similarity_confidence(a: &Paper, b: &Paper) -> bool {
    // High title similarity
    let title_a = a.title.to_lowercase().trim().to_string();
    let title_b = b.title.to_lowercase().trim().to_string();
    let similarity = jaro_winkler(&title_a, &title_b);

    if similarity >= 0.95 && authors_match(a, b) {
        return true;
    }

    // Exact normalized title match with author overlap
    if normalize_title(&title_a) == normalize_title(&title_b) && authors_match(a, b) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PaperBuilder, SourceType};

    #[test]
    fn empty_and_conflicting_dois_do_not_cause_duplicates() {
        let make = |id, title, source, doi: &str| {
            PaperBuilder::new(id, title, "https://example.com", source)
                .doi(doi)
                .build()
        };
        let empty = vec![
            make("1", "Apples", SourceType::Arxiv, " "),
            make("2", "Zebras", SourceType::OpenAlex, ""),
        ];
        let conflict = vec![
            make("1", "Identical title", SourceType::Arxiv, "10/a"),
            make("2", "Identical title", SourceType::OpenAlex, "10/b"),
        ];
        for papers in [empty, conflict] {
            assert_eq!(
                deduplicate_papers(papers.clone(), DuplicateStrategy::First).len(),
                2
            );
            assert_eq!(
                fast_deduplicate_papers(papers, DuplicateStrategy::First).len(),
                2
            );
        }
    }

    #[test]
    fn test_normalize_title() {
        assert_eq!(normalize_title("Hello, World!"), "Hello World");
        assert_eq!(normalize_title("Test   Title"), "Test Title");
        assert_eq!(normalize_title("Test: A-B/C"), "Test ABC");
        assert_eq!(normalize_title(""), "");
        assert_eq!(normalize_title("   "), "");
    }

    #[test]
    fn test_deduplicate_by_doi() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].paper_id, "1");
    }

    #[test]
    fn test_deduplicate_by_doi_case_insensitive() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/TEST")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_deduplicate_by_title() {
        let papers = vec![
            PaperBuilder::new(
                "1",
                "Machine Learning for Cats",
                "https://arxiv.org/1",
                SourceType::Arxiv,
            )
            .authors("John Doe")
            .build(),
            PaperBuilder::new(
                "2",
                "Machine Learning for Cats",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .authors("John Doe; Jane Smith")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_deduplicate_keep_last() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::Last);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].paper_id, "2");
    }

    #[test]
    fn test_deduplicate_mark_strategy() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::Mark);
        // Mark strategy should keep all papers
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_no_duplicates_same_source() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv).build(),
            PaperBuilder::new("2", "Test Paper", "https://arxiv.org/2", SourceType::Arxiv).build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_no_duplicates_different_titles() {
        let papers = vec![
            PaperBuilder::new("1", "Paper A", "https://arxiv.org/1", SourceType::Arxiv)
                .authors("John Doe")
                .build(),
            PaperBuilder::new(
                "2",
                "Paper B",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .authors("John Doe")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_no_duplicates_no_common_authors() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .authors("John Doe")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .authors("Jane Smith")
            .build(),
        ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_deduplicate_empty_list() {
        let papers = vec![];
        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 0);
    }

    #[test]
    fn test_deduplicate_single_paper() {
        let papers =
            vec![
                PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                    .build(),
            ];

        let deduped = deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_find_duplicates() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
            PaperBuilder::new("3", "Other Paper", "https://arxiv.org/3", SourceType::Arxiv).build(),
        ];

        let groups = find_duplicates(&papers);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0, 1]);
    }

    #[test]
    fn test_find_duplicates_empty() {
        let papers = vec![];
        let groups = find_duplicates(&papers);
        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn test_authors_match_no_authors() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv).build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .build(),
        ];

        let deduped = deduplicate_papers(papers.clone(), DuplicateStrategy::First);
        // Without authors, should still match on title
        assert_eq!(deduped.len(), 1);
    }

    // Tests for fast_deduplicate_papers

    #[test]
    fn test_fast_deduplicate_by_doi() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].paper_id, "1");
    }

    #[test]
    fn test_fast_deduplicate_by_title() {
        let papers = vec![
            PaperBuilder::new(
                "1",
                "Machine Learning for Cats",
                "https://arxiv.org/1",
                SourceType::Arxiv,
            )
            .authors("John Doe")
            .build(),
            PaperBuilder::new(
                "2",
                "Machine Learning for Cats",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .authors("John Doe; Jane Smith")
            .build(),
        ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_fast_deduplicate_keep_last() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::Last);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].paper_id, "2");
    }

    #[test]
    fn test_fast_deduplicate_empty() {
        let papers = vec![];
        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 0);
    }

    #[test]
    fn test_fast_deduplicate_single() {
        let papers =
            vec![
                PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                    .build(),
            ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_fast_no_duplicates_different_titles() {
        let papers = vec![
            PaperBuilder::new("1", "Paper A", "https://arxiv.org/1", SourceType::Arxiv)
                .authors("John Doe")
                .build(),
            PaperBuilder::new(
                "2",
                "Paper B",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .authors("John Doe")
            .build(),
        ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_fast_deduplicate_multiple_sources() {
        let papers = vec![
            PaperBuilder::new("1", "Test Paper", "https://arxiv.org/1", SourceType::Arxiv)
                .doi("10.1234/test")
                .build(),
            PaperBuilder::new(
                "2",
                "Test Paper",
                "https://semantic.org/2",
                SourceType::SemanticScholar,
            )
            .doi("10.1234/test")
            .build(),
            PaperBuilder::new(
                "3",
                "Test Paper",
                "https://openalex.org/3",
                SourceType::OpenAlex,
            )
            .doi("10.1234/test")
            .build(),
        ];

        let deduped = fast_deduplicate_papers(papers, DuplicateStrategy::First);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].paper_id, "1");
    }
}
