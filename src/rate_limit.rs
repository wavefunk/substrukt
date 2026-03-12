use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// Per-IP sliding window rate limiter.
#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<DashMap<String, Vec<Instant>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            requests: Arc::new(DashMap::new()),
            max_requests,
            window,
        }
    }

    /// Check if a request from the given key is allowed. Returns false if rate limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entry = self.requests.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < self.window);
        if entry.len() >= self.max_requests {
            return false;
        }
        entry.push(now);
        true
    }
}
