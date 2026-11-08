//! The allowance of a public route of a plugin (ADR-0080): a token bucket for each route, shared by
//! everyone who calls it. Nobody is signed in, and the core cannot trust an address behind a proxy
//! to tell callers apart, so the route as a whole is what is limited.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// A public route: the install and the place of the route in its manifest. Only routes that matched
/// a declared one get here, so the map holds at most the routes of the installed plugins.
pub type RouteKey = (Uuid, usize);

pub struct RouteLimiter {
    burst: f64,
    per_second: f64,
    buckets: Mutex<HashMap<RouteKey, (f64, Instant)>>,
}

impl RouteLimiter {
    pub fn new(burst: u32, per_second: u32) -> Self {
        Self {
            burst: f64::from(burst.max(1)),
            per_second: f64::from(per_second.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one call from the route's allowance, or says how long to wait for the next.
    pub fn check(&self, key: RouteKey) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: RouteKey, now: Instant) -> Result<(), Duration> {
        // The lock is never held across an await and guards plain arithmetic, so a poisoned lock
        // still holds a usable map.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (tokens, since) = buckets.entry(key).or_insert((self.burst, now));
        let earned = now.saturating_duration_since(*since).as_secs_f64() * self.per_second;
        *tokens = (*tokens + earned).min(self.burst);
        *since = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - *tokens) / self.per_second))
        }
    }
}

#[cfg(test)]
mod tests;
