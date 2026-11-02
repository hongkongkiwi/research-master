//! Mock source for testing purposes.

use async_trait::async_trait;
use std::sync::Mutex;

use crate::models::{Paper, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};

/// A mock source for testing that returns predefined responses.
#[derive(Debug, Default, Clone)]
pub struct MockSource {
    search_response: std::sync::Arc<Mutex<Option<SearchResponse>>>,
    search_calls: std::sync::Arc<Mutex<Vec<SearchQuery>>>,
}

impl MockSource {
    /// Create a new mock source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the search response to return.
    pub fn set_search_response(&self, response: SearchResponse) {
        let mut guard = self.search_response.lock().unwrap();
        *guard = Some(response);
        self.search_calls.lock().unwrap().clear();
    }

    /// Clear the configured response.
    pub fn clear_response(&self) {
        let mut guard = self.search_response.lock().unwrap();
        *guard = None;
        self.search_calls.lock().unwrap().clear();
    }

    /// Return the recorded search queries.
    pub fn search_calls(&self) -> Vec<SearchQuery> {
        self.search_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Source for MockSource {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock Source"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        self.search_calls.lock().unwrap().push(query.clone());
        let guard = self.search_response.lock().unwrap();
        match &*guard {
            Some(response) => {
                let mut page = response.clone();
                page.papers = response
                    .papers
                    .iter()
                    .skip(query.offset)
                    .take(query.max_results)
                    .cloned()
                    .collect();
                Ok(page)
            }
            None => Ok(SearchResponse::new(Vec::new(), "Mock Source", &query.query)),
        }
    }
}

/// Helper function to create a mock paper for testing.
pub fn make_paper(paper_id: &str, title: &str, source_type: SourceType) -> Paper {
    Paper::new(
        paper_id.to_string(),
        title.to_string(),
        format!("http://example.com/{}", paper_id),
        source_type,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_slices_and_records_offsets() {
        let mock = MockSource::new();
        mock.set_search_response(SearchResponse::new(
            vec![
                make_paper("1", "One", SourceType::Arxiv),
                make_paper("2", "Two", SourceType::Arxiv),
            ],
            "Mock Source",
            "test",
        ));
        let query = SearchQuery::new("test").offset(1).max_results(1);
        assert_eq!(mock.search(&query).await.unwrap().papers[0].paper_id, "2");
        assert!(mock
            .search(&query.offset(2))
            .await
            .unwrap()
            .papers
            .is_empty());
        assert_eq!(
            mock.search_calls()
                .iter()
                .map(|q| q.offset)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        mock.clear_response();
        assert!(mock.search_calls().is_empty());
    }
}
