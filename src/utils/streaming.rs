//! Async streaming utilities for large result sets.
//!
//! This module provides streaming iterators for processing large
//! search results incrementally without loading everything into memory.

use crate::models::{Paper, SearchQuery};
use crate::sources::Source;
use async_stream::stream;
use futures_util::stream::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::warn;

/// Create a stream that yields papers one at a time from paginated search results.
///
/// This allows processing large result sets without loading everything
/// into memory at once. The stream automatically handles pagination
/// and rate limiting.
pub fn paper_stream<T: Source + Clone + 'static>(
    source: T,
    query: SearchQuery,
    page_size: usize,
) -> impl Stream<Item = Paper> + Send {
    stream! {
        let rate_limit_delay = Duration::from_millis(200);
        loop {
            // Clone query for this page
            let mut page_query = query.clone();
            page_query.max_results = page_size;

            match source.search(&page_query).await {
                Ok(response) => {
                    let papers = response.papers;
                    let count = papers.len();

                    if count == 0 {
                        // No more papers
                        break;
                    }

                    // Yield each paper
                    for paper in papers {
                        yield paper;
                    }

                    // Apply rate limiting
                    if rate_limit_delay > Duration::ZERO {
                        sleep(rate_limit_delay).await;
                    }
                }
                Err(e) => {
                    warn!("Error fetching papers: {}", e);
                    break;
                }
            }
        }
    }
}

/// Create a stream that filters papers by year range.
pub fn filter_by_year<S: Stream<Item = Paper> + Send + 'static>(
    stream: S,
    min_year: Option<i32>,
    max_year: Option<i32>,
) -> FilterByYearStream<S> {
    FilterByYearStream::new(stream, min_year, max_year)
}

/// Collect all papers from a stream into a Vec.
pub async fn collect_papers<S: Stream<Item = Paper> + Send + Unpin>(mut stream: S) -> Vec<Paper> {
    let mut papers = Vec::new();
    while let Some(paper) = stream.next().await {
        papers.push(paper);
    }
    papers
}

/// A channel-based concurrent stream for parallel source searches.
///
/// This allows searching multiple sources concurrently and
/// yielding results as they arrive.
#[allow(dead_code)]
pub struct ConcurrentPaperStream {
    receiver: mpsc::Receiver<Paper>,
    pending: usize,
}

impl ConcurrentPaperStream {
    /// Create a new concurrent stream from a list of sources
    ///
    /// Searches all sources concurrently and yields papers in the
    /// order they complete.
    pub async fn from_sources<S: Source + Clone + 'static>(
        sources: Vec<S>,
        query: &SearchQuery,
        max_concurrent: usize,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let sources_len = sources.len();

        for source in sources {
            let query = query.clone();
            let sender = sender.clone();
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore never closed — this is a static Arc that lives for the function duration");
            let source = source.clone();

            tokio::spawn(async move {
                // permit is automatically dropped when this async block ends
                match source.search(&query).await {
                    Ok(response) => {
                        for paper in response.papers {
                            if sender.send(paper).await.is_err() {
                                break; // Receiver dropped
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Source search failed: {}", e);
                    }
                }
                drop(permit);
            });
        }

        // Drop sender to signal completion when all tasks finish
        drop(sender);

        Self {
            receiver,
            pending: sources_len,
        }
    }

    /// Get the next paper from any source
    pub async fn next(&mut self) -> Option<Paper> {
        self.receiver.recv().await
    }

    /// Check if more results are coming
    pub fn is_done(&self) -> bool {
        self.receiver.is_closed()
    }
}

/// Stream that limits the number of items
#[derive(Debug)]
pub struct TakeStream<S: Stream<Item = Paper>> {
    stream: S,
    remaining: usize,
}

impl<S: Stream<Item = Paper> + Unpin> TakeStream<S> {
    /// Create a new take stream
    pub fn new(stream: S, limit: usize) -> Self {
        Self {
            stream,
            remaining: limit,
        }
    }
}

impl<S: Stream<Item = Paper> + Unpin> Stream for TakeStream<S> {
    type Item = Paper;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.stream).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                self.remaining -= 1;
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Stream that skips items
#[derive(Debug)]
pub struct SkipStream<S: Stream<Item = Paper>> {
    stream: S,
    to_skip: usize,
}

impl<S: Stream<Item = Paper>> SkipStream<S> {
    /// Create a new skip stream
    pub fn new(stream: S, n: usize) -> Self {
        Self { stream, to_skip: n }
    }
}

impl<S: Stream<Item = Paper> + Unpin> Stream for SkipStream<S> {
    type Item = Paper;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    if self.to_skip > 0 {
                        self.to_skip -= 1;
                        continue;
                    }
                    return Poll::Ready(Some(item));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Stream filter for year range
#[derive(Debug)]
pub struct FilterByYearStream<S: Stream<Item = Paper>> {
    stream: S,
    min_year: Option<i32>,
    max_year: Option<i32>,
}

impl<S: Stream<Item = Paper>> FilterByYearStream<S> {
    /// Create a new year filter stream
    pub fn new(stream: S, min_year: Option<i32>, max_year: Option<i32>) -> Self {
        Self {
            stream,
            min_year,
            max_year,
        }
    }
}

impl<S: Stream<Item = Paper> + Unpin> Stream for FilterByYearStream<S> {
    type Item = Paper;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(paper)) => {
                    // Try to extract year from published_date (format: "YYYY-MM-DD" or "YYYY")
                    if let Some(year) = extract_year(&paper.published_date) {
                        if let Some(min) = this.min_year {
                            if year < min {
                                continue;
                            }
                        }
                        if let Some(max) = this.max_year {
                            if year > max {
                                continue;
                            }
                        }
                    }
                    return Poll::Ready(Some(paper));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Extract year from published_date string
fn extract_year(published_date: &Option<String>) -> Option<i32> {
    published_date.as_ref().and_then(|date| {
        // Split by both '-' and '/' and take first non-empty part
        date.split(['-', '/'])
            .next()
            .filter(|s| !s.is_empty())
            .and_then(|y| y.parse::<i32>().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Paper, SearchResponse, SourceType};
    use crate::sources::mock::MockSource;
    use futures_util::StreamExt;

    fn make_paper(paper_id: &str, title: &str, source_type: SourceType) -> Paper {
        Paper::new(
            paper_id.to_string(),
            title.to_string(),
            format!("http://example.com/{}", paper_id),
            source_type,
        )
    }

    #[tokio::test]
    async fn test_paper_stream_basic() {
        let mock = MockSource::new();
        mock.set_search_response(SearchResponse::new(
            vec![
                make_paper("1", "Paper 1", SourceType::Arxiv),
                make_paper("2", "Paper 2", SourceType::Arxiv),
                make_paper("3", "Paper 3", SourceType::Arxiv),
            ],
            "Mock Source",
            "test",
        ));

        let stream = paper_stream(mock, SearchQuery::new("test"), 10);
        let mut stream = Box::pin(stream);
        let mut papers = Vec::new();

        while let Some(paper) = stream.next().await {
            papers.push(paper);
        }

        assert_eq!(papers.len(), 3);
        assert_eq!(papers[0].paper_id, "1");
        assert_eq!(papers[1].paper_id, "2");
        assert_eq!(papers[2].paper_id, "3");
    }

    #[tokio::test]
    async fn test_paper_stream_empty() {
        let mock = MockSource::new();
        mock.set_search_response(SearchResponse::new(Vec::new(), "Mock Source", "test"));

        let stream = paper_stream(mock, SearchQuery::new("test"), 10);
        let mut stream = Box::pin(stream);
        let mut papers = Vec::new();

        while let Some(paper) = stream.next().await {
            papers.push(paper);
        }

        assert!(papers.is_empty());
    }

    #[test]
    fn test_extract_year() {
        assert_eq!(extract_year(&Some("2023-05-15".to_string())), Some(2023));
        assert_eq!(extract_year(&Some("2023".to_string())), Some(2023));
        assert_eq!(extract_year(&Some("2023/05/15".to_string())), Some(2023));
        assert_eq!(extract_year(&None), None);
        assert_eq!(extract_year(&Some("invalid".to_string())), None);
    }
}
