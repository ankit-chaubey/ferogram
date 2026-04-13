// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! MTProxy secret parsing, transport auto-selection, and TCP connect.
//!
//! | Secret prefix | Transport |
//! |---|---|
//! | 16 raw bytes | Obfuscated Abridged |
//! | `0xDD` + 16 bytes | PaddedIntermediate |
//! | `0xEE` + 16 bytes + domain | FakeTLS |

use crate::{InvocationError, TransportKind};
use tokio::net::TcpStream;

/// Decoded MTProxy configuration extracted from a proxy link.
#[derive(Clone, Debug)]
pub struct MtProxyConfig {
    /// Proxy server hostname or IP.
    pub host: String,
    /// Proxy server port.
    pub port: u16,
    /// Raw secret bytes.
    pub secret: Vec<u8>,
    /// Transport variant pass this as `config.transport`.
    pub transport: TransportKind,
}

impl MtProxyConfig {
    /// Open a TCP connection to the MTProxy host:port.
    /// The proxy forwards traffic to Telegram; do NOT also connect to a DC addr.
    pub async fn connect(&self) -> Result<TcpStream, InvocationError> {
        let addr = format!("{}:{}", self.host, self.port);
        tracing::debug!("[ferogram] MTProxy TCP connect → {addr}");
        TcpStream::connect(&addr).await.map_err(InvocationError::Io)
    }

    /// Socket address string `"host:port"`.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Parse a `tg://proxy?server=...&port=...&secret=...` or `https://t.me/proxy?...` link.
pub fn parse_proxy_link(url: &str) -> Option<MtProxyConfig> {
    let query = url
        .strip_prefix("tg://proxy?")
        .or_else(|| url.strip_prefix("https://t.me/proxy?"))?;

    let mut server = None;
    let mut port: Option<u16> = None;
    let mut secret_hex = None;

    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "server" => server = Some(v.to_string()),
                "port" => port = v.parse().ok(),
                "secret" => secret_hex = Some(v.to_string()),
                _ => {}
            }
        }
    }

    let host = server?;
    let port = port?;
    let secret = decode_secret_hex(&secret_hex?)?;
    let transport = secret_to_transport(&secret);
    Some(MtProxyConfig {
        host,
        port,
        secret,
        transport,
    })
}

fn decode_secret_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes: Option<Vec<u8>> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
            .collect();
        if let Some(b) = bytes {
            return Some(b);
        }
    }
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
}

/// Map secret prefix to the correct [`TransportKind`].
pub fn secret_to_transport(secret: &[u8]) -> TransportKind {
    match secret.first() {
        Some(&0xDD) => {
            let key = extract_key_bytes(secret, 1);
            TransportKind::PaddedIntermediate { secret: key }
        }
        Some(&0xEE) => {
            let key = extract_key_bytes(secret, 1);
            let domain = if secret.len() > 17 {
                String::from_utf8_lossy(&secret[17..]).into_owned()
            } else {
                String::new()
            };
            match key {
                Some(k) => TransportKind::FakeTls { secret: k, domain },
                None => TransportKind::Obfuscated { secret: None },
            }
        }
        _ => {
            let key = extract_key_bytes(secret, 0);
            TransportKind::Obfuscated { secret: key }
        }
    }
}

fn extract_key_bytes(secret: &[u8], offset: usize) -> Option<[u8; 16]> {
    let slice = secret.get(offset..offset + 16)?;
    let mut arr = [0u8; 16];
    arr.copy_from_slice(slice);
    Some(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_secret() {
        let url = "tg://proxy?server=1.2.3.4&port=443&secret=deadbeefdeadbeefdeadbeefdeadbeef";
        let cfg = parse_proxy_link(url).unwrap();
        assert_eq!(cfg.host, "1.2.3.4");
        assert_eq!(cfg.port, 443);
        assert!(matches!(cfg.transport, TransportKind::Obfuscated { .. }));
        assert_eq!(cfg.addr(), "1.2.3.4:443");
    }

    #[test]
    fn parse_dd_secret() {
        let url =
            "tg://proxy?server=p.example.com&port=8888&secret=dddeadbeefdeadbeefdeadbeefdeadbeef";
        let cfg = parse_proxy_link(url).unwrap();
        assert!(matches!(
            cfg.transport,
            TransportKind::PaddedIntermediate { .. }
        ));
    }

    #[test]
    fn parse_ee_secret() {
        let mut raw = vec![0xeeu8];
        raw.extend_from_slice(&[0xabu8; 16]);
        raw.extend_from_slice(b"example.com");
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let url = format!("tg://proxy?server=p.example.com&port=443&secret={hex}");
        let cfg = parse_proxy_link(&url).unwrap();
        if let TransportKind::FakeTls { domain, .. } = &cfg.transport {
            assert_eq!(domain, "example.com");
        } else {
            panic!("expected FakeTls");
        }
    }

    #[test]
    fn invalid_url_returns_none() {
        assert!(parse_proxy_link("https://example.com").is_none());
        assert!(parse_proxy_link("tg://proxy?server=x&port=443").is_none());
    }
}
