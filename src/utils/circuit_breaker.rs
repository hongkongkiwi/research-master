//! Circuit breaker pattern implementation for API resilience.
//!
//! The circuit breaker prevents cascading failures by temporarily disabling
//! requests to sources that are failing. It has three states:
//!
//! - **Closed**: Normal operation, requests pass through
//! - **Open**: Source is failing, requests are immediately rejected
//! - **Half-Open**: Testing if the source has recovered
//!
//! # Usage
//!
//! ```rust
//! use research_master::utils::{CircuitBreaker, CircuitState};
//!
//! let breaker = CircuitBreaker::new("semantic", 5, std::time::Duration::from_secs(60));
//!
//! assert_eq!(breaker.state(), CircuitState::Closed);
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation - requests pass through
    Closed,
    /// Failing - requests are rejected
    Open,
    /// Testing recovery - limited requests allowed
    HalfOpen,
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: usize,

    /// Number of successes in half-open state to close the circuit
    pub success_threshold: usize,

    /// Duration to stay open before trying half-open
    pub open_duration: Duration,
}

/// Result of a circuit breaker operation
#[derive(Debug, Clone)]
pub enum CircuitResult<T> {
    /// Operation succeeded
    Success(T),
    /// Operation failed but circuit is still closed
    Failure(String),
    /// Circuit is open, request was rejected
    Rejected(String),
    /// Circuit is open but request was allowed in half-open state
    RetryAllowed(String),
}

impl<T> CircuitResult<T> {
    /// Check if the operation was successful
    pub fn is_success(&self) -> bool {
        matches!(self, CircuitResult::Success(_))
    }

    /// Check if the result is a rejection due to open circuit
    pub fn is_rejected(&self) -> bool {
        matches!(self, CircuitResult::Rejected(_))
    }

    /// Unwrap the inner value (panics if not Success)
    pub fn unwrap(self) -> T {
        match self {
            CircuitResult::Success(v) => v,
            CircuitResult::Failure(e) => panic!("unwrap on Failure: {}", e),
            CircuitResult::Rejected(e) => panic!("unwrap on Rejected: {}", e),
            CircuitResult::RetryAllowed(e) => panic!("unwrap on RetryAllowed: {}", e),
        }
    }
}

/// Thread-safe circuit breaker implementation
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Circuit name (e.g., "arxiv", "semantic")
    name: String,

    /// Current state
    state: std::sync::atomic::AtomicU8,

    /// Number of consecutive failures
    failure_count: Arc<AtomicUsize>,

    /// Number of consecutive successes (in half-open state)
    success_count: Arc<AtomicUsize>,

    /// Time when the circuit was opened.
    open_since: std::sync::Mutex<Option<Instant>>,

    /// Configuration
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    ///
    /// - `name`: Identifier for this circuit (e.g., "arxiv")
    /// - `failure_threshold`: Failures before opening (default: 5)
    /// - `open_duration`: Time to stay open before half-open (default: 60s)
    pub fn new(name: &str, failure_threshold: usize, open_duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            state: std::sync::atomic::AtomicU8::new(CircuitState::Closed as u8),
            failure_count: Arc::new(AtomicUsize::new(0)),
            success_count: Arc::new(AtomicUsize::new(0)),
            open_since: std::sync::Mutex::new(None),
            config: CircuitBreakerConfig {
                failure_threshold,
                success_threshold: 1,
                open_duration,
            },
        }
    }

    /// Create with default settings
    pub fn default_for(name: &str) -> Self {
        Self::new(name, 5, Duration::from_secs(60))
    }

    /// Get the current state
    pub fn state(&self) -> CircuitState {
        let state = self.state.load(Ordering::SeqCst);
        let state = CircuitState::try_from(state).unwrap_or(CircuitState::Closed);

        // Check if we should transition from open to half-open
        if state == CircuitState::Open {
            if let Some(since) = self.open_time() {
                if since.elapsed() >= self.config.open_duration {
                    return CircuitState::HalfOpen;
                }
            }
        }

        state
    }

    /// Get the time when the circuit was opened
    fn open_time(&self) -> Option<Instant> {
        *self.open_since.lock().expect("circuit mutex poisoned")
    }

    /// Record a success
    pub fn record_success(&self) {
        let state = self.state();

        match state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let count = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= self.config.success_threshold {
                    // Transition back to closed
                    self.state
                        .store(CircuitState::Closed as u8, Ordering::SeqCst);
                    self.failure_count.store(0, Ordering::SeqCst);
                    self.success_count.store(0, Ordering::SeqCst);
                    *self.open_since.lock().expect("circuit mutex poisoned") = None;
                    tracing::info!(
                        "[circuit-breaker] {}: circuit closed (recovered)",
                        self.name
                    );
                }
            }
            CircuitState::Open => {
                // Shouldn't happen, but handle gracefully
            }
        }
    }

    /// Record a failure
    pub fn record_failure(&self) {
        let state = self.state();

        match state {
            CircuitState::Closed => {
                let count = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= self.config.failure_threshold {
                    // Transition to open and record the time
                    *self.open_since.lock().expect("circuit mutex poisoned") = Some(Instant::now());
                    self.state.store(CircuitState::Open as u8, Ordering::SeqCst);
                    tracing::warn!(
                        "[circuit-breaker] {}: circuit opened ({} failures)",
                        self.name,
                        count
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open goes back to open
                *self.open_since.lock().expect("circuit mutex poisoned") = Some(Instant::now());
                self.state.store(CircuitState::Open as u8, Ordering::SeqCst);
                self.success_count.store(0, Ordering::SeqCst);
                tracing::warn!(
                    "[circuit-breaker] {}: circuit reopened (failure in half-open)",
                    self.name
                );
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }

    /// Check if a request should be allowed
    pub fn can_request(&self) -> bool {
        let state = self.state();
        match state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => false,
        }
    }

    /// Execute an async operation with circuit breaker protection
    ///
    /// Returns `CircuitResult::Rejected` if the circuit is open.
    pub async fn execute<F, T, E>(&self, operation: F) -> CircuitResult<T>
    where
        F: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let state = self.state();

        match state {
            CircuitState::Closed => match operation.await {
                Ok(result) => {
                    self.record_success();
                    CircuitResult::Success(result)
                }
                Err(e) => {
                    self.record_failure();
                    CircuitResult::Failure(e.to_string())
                }
            },
            CircuitState::Open => CircuitResult::Rejected(format!(
                "circuit is open for {} (source may be temporarily unavailable)",
                self.name
            )),
            CircuitState::HalfOpen => {
                // Allow one request to test recovery
                match operation.await {
                    Ok(result) => {
                        self.record_success();
                        CircuitResult::Success(result)
                    }
                    Err(e) => {
                        self.record_failure();
                        CircuitResult::Failure(e.to_string())
                    }
                }
            }
        }
    }

    /// Reset the circuit breaker to closed state
    pub fn reset(&self) {
        self.state
            .store(CircuitState::Closed as u8, Ordering::SeqCst);
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        *self.open_since.lock().expect("circuit mutex poisoned") = None;
    }
}

impl TryFrom<u8> for CircuitState {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CircuitState::Closed),
            1 => Ok(CircuitState::Open),
            2 => Ok(CircuitState::HalfOpen),
            _ => Err(()),
        }
    }
}

/// Manager for multiple circuit breakers (one per source)
#[derive(Debug, Default)]
pub struct CircuitBreakerManager {
    breakers: Arc<std::sync::RwLock<std::collections::HashMap<String, Arc<CircuitBreaker>>>>,
}

impl CircuitBreakerManager {
    /// Create a new manager
    pub fn new() -> Self {
        Self {
            breakers: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Get or create a circuit breaker for a source
    pub fn get(&self, source_id: &str) -> Arc<CircuitBreaker> {
        {
            let read_guard = self.breakers.read().expect("RwLock poisoned");
            if let Some(breaker) = read_guard.get(source_id) {
                return Arc::clone(breaker);
            }
        }

        {
            let mut write_guard = self.breakers.write().expect("RwLock poisoned");
            // Double-check after acquiring write lock
            if let Some(breaker) = write_guard.get(source_id) {
                return Arc::clone(breaker);
            }

            let breaker = Arc::new(CircuitBreaker::default_for(source_id));
            write_guard.insert(source_id.to_string(), Arc::clone(&breaker));
            breaker
        }
    }

    /// Reset all circuit breakers
    pub fn reset_all(&self) {
        let guard = self.breakers.write().expect("RwLock poisoned");
        for breaker in guard.values() {
            breaker.reset();
        }
    }

    /// Get status of all circuit breakers
    pub fn status(&self) -> Vec<(String, CircuitState, bool)> {
        let guard = self.breakers.read().expect("RwLock poisoned");
        guard
            .iter()
            .map(|(name, breaker)| (name.clone(), breaker.state(), breaker.can_request()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_circuit_breaker_closed_by_default() {
        let breaker = CircuitBreaker::default_for("test");
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.can_request());
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let breaker = Arc::new(CircuitBreaker::new("test", 3, Duration::from_secs(60)));

        // Record 2 failures - should still be closed
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.can_request());

        // Record 3rd failure - should open
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        assert!(!breaker.can_request());
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_resets() {
        let breaker = Arc::new(CircuitBreaker::new("test", 3, Duration::from_secs(60)));

        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.failure_count.load(Ordering::SeqCst), 2);

        breaker.record_success();
        assert_eq!(breaker.failure_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_execute_success() {
        let breaker = Arc::new(CircuitBreaker::new("test", 3, Duration::from_secs(60)));

        let result = breaker.execute(async { Ok::<i32, &str>(42) }).await;
        assert!(result.is_success());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_circuit_breaker_execute_rejected() {
        let breaker = Arc::new(CircuitBreaker::new("test", 1, Duration::from_secs(60)));

        // Open the circuit
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Execute should be rejected
        let result = breaker.execute(async { Ok::<i32, &str>(42) }).await;
        assert!(result.is_rejected());
    }

    #[tokio::test]
    async fn test_timed_recovery_and_failed_probe_reopens() {
        let breaker = CircuitBreaker::new("test", 1, Duration::from_millis(20));
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_manager() {
        let manager = CircuitBreakerManager::new();

        // Get two circuit breakers
        let breaker1 = manager.get("source1");
        let breaker2 = manager.get("source2");
        let breaker1_again = manager.get("source1");

        // Should be the same instance
        assert!(Arc::ptr_eq(&breaker1, &breaker1_again));
        // Different sources should be different
        assert!(!Arc::ptr_eq(&breaker1, &breaker2));
    }

    #[test]
    fn test_manager_status() {
        let manager = CircuitBreakerManager::new();

        let _ = manager.get("arxiv");
        let _ = manager.get("semantic");

        let status = manager.status();
        assert_eq!(status.len(), 2);
        assert!(status
            .iter()
            .all(|(_, state, _)| *state == CircuitState::Closed));
    }
}
