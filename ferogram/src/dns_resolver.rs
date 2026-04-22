// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! DNS-over-HTTPS resolver.
//!
//! When system DNS is poisoned or blocked (common in Iran, Russia, China),
//! this resolver queries **Mozilla Cloudflare** and **Google** DoH endpoints
//! directly over HTTPS to obtain Telegram DC IP addresses.
//!
//! # How it works
//!
//! 1. Two HTTPS requests are fired - one to Mozilla, one to Google DoH.
//! 2. Whichever responds first with valid data wins; the other is cancelled.
//! 3. Results are cached per domain for the TTL reported by the server
//!    (clamped to 10 s - 300 s).
//! 4. Both A (IPv4) and AAAA (IPv6) records are resolved and merged.
//!
//! # Usage
//!
//! ```rust,no_run
//! use ferogram::dns_resolver::DnsResolver;
//!
//! #[tokio::main]
//! async fn main() {
//!     let resolver = DnsResolver::new();
//!     let ips = resolver.resolve("venus.web.telegram.org").await;
//!     println!("IPs: {ips:?}");
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Minimum TTL to honour from server (10 s).
const TTL_MIN: Duration = Duration::from_secs(10);
/// Maximum TTL to honour from server (300 s).
const TTL_MAX: Duration = Duration::from_secs(300);

// DoH endpoints
const GOOGLE_DOH_URL: &str = "https://dns.google.com/resolve";
const MOZILLA_DOH_URL: &str = "https://mozilla.cloudflare-dns.com/dns-query";

// Randomised padding range: 13–128 chars.
const PAD_MIN: usize = 13;
const PAD_MAX: usize = 128;

// Cache entry

#[derive(Clone, Debug)]
struct CacheEntry {
    ips: Vec<String>,
    expires_at: Instant,
}

// Public types

/// Async DNS-over-HTTPS resolver with per-domain TTL cache.
///
/// Clone-cheap: the underlying cache and HTTP client are `Arc`-shared.
#[derive(Clone)]
pub struct DnsResolver {
    client: reqwest::Client,
    /// `(domain, is_ipv6)` → cached answer.
    cache: Arc<Mutex<HashMap<(String, bool), CacheEntry>>>,
}

impl std::fmt::Debug for DnsResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsResolver").finish_non_exhaustive()
    }
}

impl DnsResolver {
    /// Create a new resolver.
    ///
    /// The internal `reqwest::Client` bypasses system proxy settings so that
    /// DoH requests always reach the public DNS resolvers directly.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // Never proxy DoH requests - they are the fallback *for* blocked networks.
            .no_proxy()
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                 AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/124.0.0.0 Safari/537.36",
            )
            .build()
            .expect("DnsResolver: failed to build reqwest client");

        Self {
            client,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // Public API

    /// Resolve `domain` to a list of IPv4 + IPv6 addresses.
    ///
    /// Returns an empty `Vec` if resolution fails (callers should fall back
    /// to hardcoded DC addresses or [`crate::special_config`]).
    pub async fn resolve(&self, domain: &str) -> Vec<String> {
        let (v4, v6) = tokio::join!(
            self.resolve_type(domain, false),
            self.resolve_type(domain, true),
        );
        let mut ips = v4;
        ips.extend(v6);
        ips
    }

    /// Resolve `domain` for IPv4 only.
    pub async fn resolve_v4(&self, domain: &str) -> Vec<String> {
        self.resolve_type(domain, false).await
    }

    /// Resolve `domain` for IPv6 only.
    pub async fn resolve_v6(&self, domain: &str) -> Vec<String> {
        self.resolve_type(domain, true).await
    }

    // Internal

    async fn resolve_type(&self, domain: &str, ipv6: bool) -> Vec<String> {
        // 1. Check cache.
        {
            let cache = self.cache.lock().await;
            if let Some(entry) = cache.get(&(domain.to_owned(), ipv6))
                && entry.expires_at > Instant::now()
            {
                tracing::debug!(
                    "[dns] cache hit: {} {} → {:?}",
                    domain,
                    if ipv6 { "AAAA" } else { "A" },
                    entry.ips
                );
                return entry.ips.clone();
            }
        }

        // 2. Race Mozilla vs Google DoH - first valid answer wins.
        let rrtype = if ipv6 { 28u8 } else { 1u8 }; // AAAA=28, A=1
        let pad = generate_padding();

        let google_fut = self.query_google(domain, rrtype, &pad);
        let mozilla_fut = self.query_mozilla(domain, rrtype, &pad);

        let result = tokio::select! {
            r = google_fut  => r,
            r = mozilla_fut => r,
        };

        let entries = match result {
            Ok(v) if !v.is_empty() => v,
            _ => {
                // Retry whichever lost the race.
                let pad2 = generate_padding();
                let r = tokio::join!(
                    self.query_google(domain, rrtype, &pad2),
                    self.query_mozilla(domain, rrtype, &pad2),
                );
                match (r.0, r.1) {
                    (Ok(v), _) if !v.is_empty() => v,
                    (_, Ok(v)) if !v.is_empty() => v,
                    _ => {
                        tracing::warn!(
                            "[dns] DoH resolution failed for {} {}",
                            domain,
                            if ipv6 { "AAAA" } else { "A" }
                        );
                        return vec![];
                    }
                }
            }
        };

        // 3. Pick the minimum TTL, clamp, then cache.
        let min_ttl = entries
            .iter()
            .map(|e| e.ttl)
            .min()
            .unwrap_or(TTL_MIN.as_secs());
        let ttl = Duration::from_secs(min_ttl).max(TTL_MIN).min(TTL_MAX);

        let ips: Vec<String> = entries.into_iter().map(|e| e.data).collect();

        tracing::debug!(
            "[dns] resolved {} {} → {:?} (TTL={:?})",
            domain,
            if ipv6 { "AAAA" } else { "A" },
            ips,
            ttl
        );

        {
            let mut cache = self.cache.lock().await;
            cache.insert(
                (domain.to_owned(), ipv6),
                CacheEntry {
                    ips: ips.clone(),
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        ips
    }

    // DoH back-ends

    /// Query Google Public DNS over HTTPS.
    ///
    /// Endpoint: `https://dns.google.com/resolve?name=<domain>&type=<A|AAAA>&random_padding=<pad>`
    async fn query_google(
        &self,
        domain: &str,
        rrtype: u8,
        padding: &str,
    ) -> Result<Vec<DnsEntry>, DohError> {
        let url = format!("{GOOGLE_DOH_URL}?name={domain}&type={rrtype}&random_padding={padding}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DohError::Http(e.to_string()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| DohError::Http(e.to_string()))?;
        parse_doh_json(&bytes, Some(rrtype as u32))
    }

    /// Query Mozilla Cloudflare DNS over HTTPS.
    ///
    /// Endpoint: `https://mozilla.cloudflare-dns.com/dns-query?name=<domain>&type=<A|AAAA>&random_padding=<pad>`
    async fn query_mozilla(
        &self,
        domain: &str,
        rrtype: u8,
        padding: &str,
    ) -> Result<Vec<DnsEntry>, DohError> {
        let url = format!("{MOZILLA_DOH_URL}?name={domain}&type={rrtype}&random_padding={padding}");
        let resp = self
            .client
            .get(&url)
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| DohError::Http(e.to_string()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| DohError::Http(e.to_string()))?;
        parse_doh_json(&bytes, Some(rrtype as u32))
    }
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

// JSON parsing

/// One record from the DoH JSON `Answer` array.
#[derive(Debug, Clone)]
struct DnsEntry {
    /// IP address (as string) or TXT data.
    data: String,
    /// TTL in seconds.
    ttl: u64,
}

#[derive(Debug)]
enum DohError {
    Http(String),
    Parse(String),
}

impl std::fmt::Display for DohError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP: {e}"),
            Self::Parse(e) => write!(f, "parse: {e}"),
        }
    }
}

/// Parse a DoH JSON response body.
///
/// Expected shape:
/// ```json
/// { "Answer": [ { "data": "1.2.3.4", "TTL": 300, "type": 1 } ] }
/// ```
fn parse_doh_json(bytes: &[u8], type_filter: Option<u32>) -> Result<Vec<DnsEntry>, DohError> {
    if bytes.is_empty() {
        return Err(DohError::Parse("empty response".into()));
    }

    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| DohError::Parse(format!("JSON: {e}")))?;

    let answer = match value.get("Answer") {
        Some(a) => a,
        None => return Ok(vec![]), // NXDOMAIN or no records - not an error
    };

    let arr = answer
        .as_array()
        .ok_or_else(|| DohError::Parse("Answer not an array".into()))?;

    let mut entries = Vec::new();
    for item in arr {
        // Optional type filter.
        if let Some(expected) = type_filter
            && let Some(t) = item.get("type").and_then(|v| v.as_u64())
            && t != expected as u64
        {
            continue;
        }

        let data = match item.get("data").and_then(|v| v.as_str()) {
            Some(s) => s.to_owned(),
            None => continue,
        };

        let ttl = item
            .get("TTL")
            .and_then(|v| v.as_u64())
            .unwrap_or(TTL_MIN.as_secs());

        entries.push(DnsEntry { data, ttl });
    }

    Ok(entries)
}

// Padding helper for DNS-over-HTTPS queries.

fn generate_padding() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    // Length: PAD_MIN..=PAD_MAX
    let mut len_byte = [0u8; 1];
    getrandom::getrandom(&mut len_byte).unwrap();
    let len = PAD_MIN + (len_byte[0] as usize % (PAD_MAX - PAD_MIN + 1));
    let mut result = String::with_capacity(len);
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).unwrap();
    for b in buf {
        result.push(CHARSET[b as usize % CHARSET.len()] as char);
    }
    result
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_google_doh_response() {
        let json = r#"{
            "Status": 0,
            "Answer": [
                { "name": "venus.web.telegram.org", "type": 1, "TTL": 120, "data": "149.154.167.51" },
                { "name": "venus.web.telegram.org", "type": 1, "TTL": 120, "data": "149.154.167.91" }
            ]
        }"#;
        let entries = parse_doh_json(json.as_bytes(), Some(1)).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].data, "149.154.167.51");
        assert_eq!(entries[0].ttl, 120);
    }

    #[test]
    fn parse_filters_by_type() {
        let json = r#"{ "Answer": [
            { "type": 1,  "TTL": 60, "data": "1.2.3.4" },
            { "type": 28, "TTL": 60, "data": "::1" }
        ]}"#;
        let v4 = parse_doh_json(json.as_bytes(), Some(1)).unwrap();
        assert_eq!(v4.len(), 1);
        assert_eq!(v4[0].data, "1.2.3.4");

        let v6 = parse_doh_json(json.as_bytes(), Some(28)).unwrap();
        assert_eq!(v6.len(), 1);
        assert_eq!(v6[0].data, "::1");
    }

    #[test]
    fn parse_empty_answer_ok() {
        let json = r#"{ "Status": 3 }"#; // NXDOMAIN
        let entries = parse_doh_json(json.as_bytes(), None).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn padding_length_in_range() {
        for _ in 0..20 {
            let p = generate_padding();
            assert!(
                p.len() >= PAD_MIN && p.len() <= PAD_MAX,
                "bad len {}",
                p.len()
            );
        }
    }
}
