//! Protection against saturation (ADR-014): inspection budget, upload
//! concurrency, and rate limiting per principal and per IP.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::connect_info::MockConnectInfo;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use onepack_nuget::InspectionLimits;

use crate::AppState;
use crate::auth::Surface;
use crate::nuget_api::ApiError;

/// Configurable server limits.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum compressed size of a package.
    pub max_package_bytes: u64,
    pub inspection: InspectionLimits,
    /// Simultaneous uploads; those that exceed get `503` without reading the body.
    pub max_concurrent_uploads: usize,
    /// Simultaneous package inspections, on threads separate from those serving downloads.
    pub max_concurrent_inspections: usize,
    /// Maximum time to receive an upload body.
    pub upload_timeout: Duration,
    pub principal_rate: RateLimit,
    pub ip_rate: RateLimit,
}

impl Default for Limits {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            max_package_bytes: 100 << 20,
            inspection: InspectionLimits::default(),
            max_concurrent_uploads: 8,
            max_concurrent_inspections: cpus.div_ceil(2),
            upload_timeout: Duration::from_secs(600),
            principal_rate: RateLimit::new(50, 1000),
            ip_rate: RateLimit::new(100, 2000),
        }
    }
}

/// Token bucket: `per_second` sustained with bursts of up to `burst`. `per_second = 0`
/// disables the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub per_second: u32,
    pub burst: u32,
}

impl RateLimit {
    pub const fn new(per_second: u32, burst: u32) -> Self {
        Self { per_second, burst }
    }

    pub const fn disabled() -> Self {
        Self::new(0, 0)
    }
}

/// Maximum number of remembered keys. When exceeded, full buckets (inactive clients)
/// are forgotten, so that an avalanche of different IPs does not grow memory without bound.
const MAX_KEYS: usize = 100_000;

struct Bucket {
    tokens: f64,
    updated: Instant,
}

pub struct RateLimiter<K> {
    limit: RateLimit,
    buckets: Mutex<HashMap<K, Bucket>>,
}

impl<K: Eq + Hash> RateLimiter<K> {
    pub fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Consumes a token. If none remain, returns how long to wait until the next one.
    pub fn check(&self, key: K) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: K, now: Instant) -> Result<(), Duration> {
        if self.limit.per_second == 0 {
            return Ok(());
        }
        let rate = f64::from(self.limit.per_second);
        let burst = f64::from(self.limit.burst.max(1));
        let mut buckets = self.buckets.lock().expect("mutex not poisoned");
        if buckets.len() >= MAX_KEYS && !buckets.contains_key(&key) {
            buckets.retain(|_, b| {
                b.tokens + now.duration_since(b.updated).as_secs_f64() * rate < burst
            });
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: burst,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate).min(burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / rate))
        }
    }
}

/// Per-IP limit, before authenticating: it also throttles attempts with invalid
/// credentials. The IP is the socket's; behind a reverse proxy all requests
/// share the proxy's IP (ADR-018: `X-Forwarded-For` is not trusted), so there it is advisable
/// to raise this limit or disable it and rely on the per-principal limit.
pub async fn limit_by_ip(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let extensions = req.extensions();
    let ip: Option<IpAddr> = extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
        .or_else(|| {
            extensions
                .get::<MockConnectInfo<SocketAddr>>()
                .map(|MockConnectInfo(addr)| addr.ip())
        });
    if let Some(ip) = ip
        && let Err(retry_after) = state.ip_limiter.check(ip)
    {
        tracing::warn!(%ip, "per-IP rate limit reached");
        let surface = Surface::of(req.uri().path());
        return ApiError::RateLimited(retry_after).render(surface);
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_allows_bursts_then_refills() {
        let limiter = RateLimiter::new(RateLimit::new(10, 3));
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check_at("a", t0).is_ok());
        }
        let wait = limiter.check_at("a", t0).unwrap_err();
        assert!(wait <= Duration::from_millis(100), "{wait:?}");
        // Another key has its own bucket.
        assert!(limiter.check_at("b", t0).is_ok());
        // After 100 ms there is one token again.
        assert!(
            limiter
                .check_at("a", t0 + Duration::from_millis(100))
                .is_ok()
        );
        assert!(
            limiter
                .check_at("a", t0 + Duration::from_millis(100))
                .is_err()
        );
    }

    #[test]
    fn disabled_limit_never_rejects() {
        let limiter = RateLimiter::new(RateLimit::disabled());
        let t0 = Instant::now();
        for _ in 0..10_000 {
            assert!(limiter.check_at(1, t0).is_ok());
        }
    }

    #[test]
    fn memory_is_bounded() {
        let limiter = RateLimiter::new(RateLimit::new(1000, 1));
        let t0 = Instant::now();
        for key in 0..MAX_KEYS {
            limiter.check_at(key, t0).unwrap();
        }
        // A second later all buckets are full and can be forgotten.
        limiter
            .check_at(MAX_KEYS, t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(limiter.buckets.lock().unwrap().len(), 1);
    }
}
