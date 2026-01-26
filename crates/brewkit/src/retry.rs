//! Retry logic with exponential backoff for transient errors.

use crate::error::{Error, Result};
use crate::types::RetryConfig;
use std::thread;

/// Callback trait for retry progress notifications.
pub trait RetryCallback {
    /// Called when an operation is being retried.
    ///
    /// # Arguments
    /// * `attempt` - Current attempt number (1-indexed)
    /// * `max_attempts` - Maximum number of attempts
    /// * `error` - The error that triggered the retry
    /// * `delay_secs` - Seconds until next attempt
    fn on_retry(&self, attempt: u32, max_attempts: u32, error: &Error, delay_secs: u64);
}

/// No-op callback that does nothing.
pub struct NoCallback;

impl RetryCallback for NoCallback {
    fn on_retry(&self, _attempt: u32, _max_attempts: u32, _error: &Error, _delay_secs: u64) {}
}

/// Callback that prints retry information to stderr.
pub struct PrintCallback;

impl RetryCallback for PrintCallback {
    fn on_retry(&self, attempt: u32, max_attempts: u32, error: &Error, delay_secs: u64) {
        eprintln!(
            "Attempt {}/{} failed: {}. Retrying in {}s...",
            attempt, max_attempts, error, delay_secs
        );
    }
}

/// Execute an operation with retry logic.
///
/// Retries the operation if it returns a retryable error, using exponential
/// backoff between attempts.
///
/// # Arguments
/// * `config` - Retry configuration
/// * `callback` - Optional callback for retry notifications
/// * `operation` - The operation to execute
///
/// # Returns
/// The result of the operation, or the last error if all attempts failed.
pub fn with_retry<T, F>(
    config: &RetryConfig,
    callback: Option<&dyn RetryCallback>,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut last_error: Option<Error> = None;

    for attempt in 0..config.max_attempts {
        match operation() {
            Ok(result) => return Ok(result),
            Err(e) => {
                // If error is not retryable, return immediately
                if !e.is_retryable() {
                    return Err(e);
                }

                // If this was the last attempt, return the error
                if attempt + 1 >= config.max_attempts {
                    last_error = Some(e);
                    break;
                }

                // Calculate delay and notify callback
                let delay = config.delay_for_attempt(attempt);
                let delay_secs = delay.as_secs();

                if let Some(cb) = callback {
                    cb.on_retry(attempt + 1, config.max_attempts, &e, delay_secs);
                }

                // Wait before retry
                thread::sleep(delay);

                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::Other("retry exhausted".to_string())))
}

/// Execute an operation with retry using default config and no callback.
pub fn with_retry_simple<T, F>(operation: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    with_retry(&RetryConfig::default(), None, operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn test_with_retry_success_first_try() {
        let config = RetryConfig::no_retry();
        let result = with_retry(&config, None, || Ok::<_, Error>(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_retry_non_retryable_error() {
        let config = RetryConfig::default();
        let attempts = Rc::new(Cell::new(0));
        let attempts_clone = attempts.clone();

        let result: Result<()> = with_retry(&config, None, || {
            attempts_clone.set(attempts_clone.get() + 1);
            Err(Error::NotFound {
                name: "foo".to_string(),
            })
        });

        assert!(result.is_err());
        // Should only try once since NotFound is not retryable
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn test_with_retry_eventual_success() {
        let config = RetryConfig {
            max_attempts: 3,
            base_delay: std::time::Duration::from_millis(1),
            backoff_factor: 1.0,
            max_delay: std::time::Duration::from_millis(10),
        };
        let attempts = Rc::new(Cell::new(0));
        let attempts_clone = attempts.clone();

        let result = with_retry(&config, None, || {
            let current = attempts_clone.get();
            attempts_clone.set(current + 1);
            if current < 2 {
                Err(Error::Network {
                    message: "timeout".to_string(),
                })
            } else {
                Ok(42)
            }
        });

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn test_with_retry_all_attempts_fail() {
        let config = RetryConfig {
            max_attempts: 3,
            base_delay: std::time::Duration::from_millis(1),
            backoff_factor: 1.0,
            max_delay: std::time::Duration::from_millis(10),
        };
        let attempts = Rc::new(Cell::new(0));
        let attempts_clone = attempts.clone();

        let result: Result<()> = with_retry(&config, None, || {
            attempts_clone.set(attempts_clone.get() + 1);
            Err(Error::Network {
                message: "timeout".to_string(),
            })
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn test_callback_invoked() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingCallback(Arc<AtomicU32>);
        impl RetryCallback for CountingCallback {
            fn on_retry(&self, _: u32, _: u32, _: &Error, _: u64) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let config = RetryConfig {
            max_attempts: 3,
            base_delay: std::time::Duration::from_millis(1),
            backoff_factor: 1.0,
            max_delay: std::time::Duration::from_millis(10),
        };

        let callback_count = Arc::new(AtomicU32::new(0));
        let callback = CountingCallback(callback_count.clone());

        let _: Result<()> = with_retry(&config, Some(&callback), || {
            Err(Error::Network {
                message: "timeout".to_string(),
            })
        });

        // Callback should be called for each retry (not the first attempt, not the last)
        assert_eq!(callback_count.load(Ordering::SeqCst), 2);
    }
}
