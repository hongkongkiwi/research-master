//! Retry utilities with exponential backoff for resilient API calls.

use std::time::Duration;
use tokio::time::{sleep, timeout};

use crate::sources::SourceError;

/// Configuration for retry behavior
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Maximum total time to spend on retries (including delays)
    pub max_total_time: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            max_total_time: Duration::from_secs(120),
        }
    }
}

/// Transient errors that should trigger a retry
#[derive(Debug, Clone, PartialEq)]
pub enum TransientError {
    /// Network connectivity issues
    Network,
    /// Rate limit exceeded (with optional retry-after seconds)
    RateLimit(Option<u64>),
    /// Server error (5xx)
    ServerError,
    /// Service unavailable (503)
    ServiceUnavailable,
    /// Gateway timeout (504)
    GatewayTimeout,
    /// Too many requests (429)
    TooManyRequests,
    /// Request timeout
    Timeout,
}

impl TransientError {
    /// Check if a reqwest error represents a transient error
    pub fn from_reqwest_error(err: &reqwest::Error) -> Option<Self> {
        if err.is_timeout() {
            return Some(TransientError::Timeout);
        }
        if err.is_connect() {
            return Some(TransientError::Network);
        }

        if let Some(status) = err.status() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Some(TransientError::TooManyRequests);
            }

            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Some(TransientError::ServiceUnavailable);
            }

            if status == reqwest::StatusCode::GATEWAY_TIMEOUT {
                return Some(TransientError::GatewayTimeout);
            }

            if status.is_server_error() {
                return Some(TransientError::ServerError);
            }
        }

        None
    }

    /// Check if a SourceError represents a transient error
    pub fn from_source_error(err: &SourceError) -> Option<Self> {
        match err {
            SourceError::RateLimit => Some(TransientError::RateLimit(None)),
            SourceError::Network(_) => Some(TransientError::Network),
            SourceError::Api(msg) => {
                // Heuristic: check for common transient error patterns in messages
                let msg_lower = msg.to_lowercase();
                if msg_lower.contains("timeout") {
                    Some(TransientError::Timeout)
                } else if msg_lower.contains("service unavailable")
                    || msg_lower.contains("temporarily unavailable")
                {
                    Some(TransientError::ServiceUnavailable)
                } else if msg_lower.contains("status: 429") || msg_lower.contains("rate-limit") {
                    Some(TransientError::TooManyRequests)
                } else if msg_lower.contains("status: 503") {
                    Some(TransientError::ServiceUnavailable)
                } else if msg_lower.contains("status: 504") {
                    Some(TransientError::GatewayTimeout)
                } else if msg_lower.contains("status: 500") || msg_lower.contains("status: 502") {
                    Some(TransientError::ServerError)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get the recommended delay for this error
    pub fn recommended_delay(&self) -> Duration {
        match self {
            TransientError::RateLimit(Some(seconds)) => Duration::from_secs(*seconds + 1),
            TransientError::RateLimit(None) => Duration::from_secs(61),
            TransientError::TooManyRequests => Duration::from_secs(61),
            TransientError::ServiceUnavailable => Duration::from_secs(10),
            TransientError::GatewayTimeout => Duration::from_secs(5),
            TransientError::Timeout => Duration::from_secs(2),
            TransientError::Network => Duration::from_secs(2),
            TransientError::ServerError => Duration::from_secs(2),
        }
    }
}

/// Result of a retry operation
pub enum RetryResult<T> {
    /// Operation succeeded
    Success(T),
    /// Operation failed with a transient error after all retries
    TransientFailure(SourceError, TransientError, u32),
    /// Operation failed with a permanent error
    PermanentFailure(SourceError),
}

/// Execute an async operation with retry logic
///
/// # Arguments
///
/// * `config` - Retry configuration
/// * `operation` - The async operation to execute
///
/// # Returns
///
/// The result of the operation, or an error after all retries are exhausted
pub async fn with_retry<T, F, Fut>(config: RetryConfig, operation: F) -> Result<T, SourceError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SourceError>>,
{
    let mut attempts = 0;
    let deadline = tokio::time::Instant::now() + config.max_total_time;
    let mut operation = operation;

    loop {
        attempts += 1;

        match timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            operation(),
        )
        .await
        {
            Ok(Ok(result)) => {
                // Success
                if attempts > 1 {
                    tracing::info!(
                        "Operation succeeded on attempt {} after {} transient failures",
                        attempts,
                        attempts - 1
                    );
                }
                return Ok(result);
            }
            Ok(Err(error)) => {
                // Check if this is a transient error
                if let Some(transient) = TransientError::from_source_error(&error) {
                    // Calculate delay with exponential backoff
                    let delay = if attempts == 1 {
                        config.initial_delay
                    } else {
                        let exp_delay = config.initial_delay.as_secs_f64()
                            * config.backoff_multiplier.powf(attempts as f64 - 1.0);
                        let delay_secs = exp_delay.min(config.max_delay.as_secs_f64());
                        Duration::from_secs_f64(delay_secs)
                    };

                    // Also consider error-specific recommended delay
                    let delay = std::cmp::max(delay, transient.recommended_delay());

                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if attempts >= config.max_attempts || delay >= remaining {
                        tracing::warn!(
                            "Operation failed after {} attempts (total elapsed: {:?}): {}",
                            attempts,
                            config.max_total_time.saturating_sub(remaining),
                            error
                        );
                        return Err(error);
                    }

                    tracing::debug!(
                        "Transient error on attempt {}: {:?}, retrying in {:?}",
                        attempts,
                        transient,
                        delay
                    );

                    sleep(delay).await;
                    continue;
                } else {
                    // Permanent error - return immediately
                    return Err(error);
                }
            }
            Err(_) => {
                // Timeout of the entire operation
                let error = SourceError::Network("Operation timed out".to_string());
                if attempts >= config.max_attempts {
                    return Err(error);
                }

                let delay = config.initial_delay;
                if delay >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                    return Err(error);
                }

                tracing::debug!(
                    "Operation timed out, attempt {}/{}",
                    attempts,
                    config.max_attempts
                );
                sleep(delay).await;
            }
        }
    }
}

/// Execute an async operation with retry logic that returns RetryResult
///
/// This provides more detailed information about failures for callers that need it
pub async fn with_retry_detailed<T, F, Fut>(config: RetryConfig, operation: F) -> RetryResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SourceError>>,
{
    let mut attempts = 0;
    let deadline = tokio::time::Instant::now() + config.max_total_time;
    let mut operation = operation;

    loop {
        attempts += 1;

        match timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            operation(),
        )
        .await
        {
            Ok(Ok(result)) => {
                return RetryResult::Success(result);
            }
            Ok(Err(error)) => {
                if let Some(transient) = TransientError::from_source_error(&error) {
                    let delay = if attempts == 1 {
                        config.initial_delay
                    } else {
                        let exp_delay = config.initial_delay.as_secs_f64()
                            * config.backoff_multiplier.powf(attempts as f64 - 1.0);
                        Duration::from_secs_f64(exp_delay.min(config.max_delay.as_secs_f64()))
                    };

                    let delay = std::cmp::max(delay, transient.recommended_delay());
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if attempts >= config.max_attempts || delay >= remaining {
                        return RetryResult::TransientFailure(error, transient, attempts);
                    }

                    sleep(delay).await;
                    continue;
                } else {
                    return RetryResult::PermanentFailure(error);
                }
            }
            Err(_) => {
                let error = SourceError::Network("Operation timed out".to_string());
                if attempts >= config.max_attempts {
                    return RetryResult::TransientFailure(error, TransientError::Timeout, attempts);
                }

                let delay = config.initial_delay;
                if delay >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                    return RetryResult::TransientFailure(error, TransientError::Timeout, attempts);
                }
                sleep(delay).await;
            }
        }
    }
}

/// Create a default retry configuration optimized for external APIs
pub fn api_retry_config() -> RetryConfig {
    RetryConfig {
        max_attempts: 5,
        initial_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(120),
        backoff_multiplier: 2.0,
        max_total_time: Duration::from_secs(300),
    }
}

/// Create a retry configuration for sources with strict rate limits
pub fn strict_rate_limit_retry_config() -> RetryConfig {
    RetryConfig {
        max_attempts: 3,
        initial_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(120),
        backoff_multiplier: 2.0,
        max_total_time: Duration::from_secs(180),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[tokio::test]
    async fn test_retry_success_first_try() {
        let config = RetryConfig::default();
        let call_count = Rc::new(RefCell::new(0));

        let result = {
            let call_count = call_count.clone();
            with_retry(config, move || {
                let call_count = call_count.clone();
                async move {
                    *call_count.borrow_mut() += 1;
                    Ok("success")
                }
            })
        }
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(*call_count.borrow(), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        // Use Network error which has 2s recommended delay, so we need longer max_total_time
        let config = RetryConfig {
            max_attempts: 4, // 4 attempts = 3 retries + final attempt
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_total_time: Duration::from_secs(10),
        };
        let call_count = Rc::new(RefCell::new(0));

        let result = {
            let call_count = call_count.clone();
            with_retry(config, move || {
                let call_count = call_count.clone();
                async move {
                    *call_count.borrow_mut() += 1;
                    let count = *call_count.borrow();
                    if count < 3 {
                        // Fail on attempts 1 and 2
                        Err(SourceError::Network("temporary error".to_string()))
                    } else {
                        // Succeed on attempt 3
                        Ok("success")
                    }
                }
            })
        }
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(*call_count.borrow(), 3);
    }

    #[tokio::test]
    async fn test_retry_returns_permanent_error() {
        let config = RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(50),
            backoff_multiplier: 2.0,
            max_total_time: Duration::from_secs(5),
        };
        let call_count = Rc::new(RefCell::new(0));

        let result: Result<&str, SourceError> = {
            let call_count = call_count.clone();
            with_retry(config, move || {
                let call_count = call_count.clone();
                async move {
                    *call_count.borrow_mut() += 1;
                    Err(SourceError::NotFound("not found".to_string()))
                }
            })
        }
        .await;

        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                SourceError::NotFound(_) => {} // Expected
                _ => panic!("Expected NotFound error"),
            }
        }
        assert_eq!(*call_count.borrow(), 1); // Should not retry on permanent error
    }

    #[test]
    fn test_transient_source_status_messages() {
        for status in [429, 500, 502, 503, 504] {
            let error = SourceError::Api(format!("Source API returned status: {status}"));
            assert!(TransientError::from_source_error(&error).is_some());
        }
        assert!(
            TransientError::from_source_error(&SourceError::Api("RATE-LIMIT reached".into()))
                .is_some()
        );
        assert!(TransientError::from_source_error(&SourceError::Api(
            "API returned status: 400".into()
        ))
        .is_none());
    }

    #[test]
    fn test_transient_error_detection() {
        // Test rate limit detection
        let rate_limit_error = SourceError::RateLimit;
        assert!(TransientError::from_source_error(&rate_limit_error).is_some());

        // Test network error detection
        let network_error = SourceError::Network("connection refused".to_string());
        assert!(TransientError::from_source_error(&network_error).is_some());

        // Test non-transient error
        let parse_error = SourceError::Parse("invalid json".to_string());
        assert!(TransientError::from_source_error(&parse_error).is_none());
    }

    #[test]
    fn test_recommended_delay() {
        assert_eq!(
            TransientError::RateLimit(Some(30)).recommended_delay(),
            Duration::from_secs(31)
        );

        assert_eq!(
            TransientError::RateLimit(None).recommended_delay(),
            Duration::from_secs(61)
        );

        assert_eq!(
            TransientError::ServiceUnavailable.recommended_delay(),
            Duration::from_secs(10)
        );

        assert_eq!(
            TransientError::Network.recommended_delay(),
            Duration::from_secs(2)
        );
    }
}
