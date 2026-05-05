// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

// MtProxyConfig has moved to ferogram-connect. Re-export for backward compatibility.
pub use ferogram_connect::MtProxyConfig;

/// Parse a Telegram MTProxy link (`https://t.me/proxy?...` or `tg://proxy?...`).
/// Returns `None` if the URL is invalid or not an MTProxy link.
pub fn parse_proxy_link(url: &str) -> Option<MtProxyConfig> {
    use ferogram_connect::TransportKind;
    let (query, port_str, host) = if let Some(rest) = url
        .strip_prefix("https://t.me/proxy?")
        .or_else(|| url.strip_prefix("tg://proxy?"))
    {
        let params: std::collections::HashMap<_, _> = rest
            .split('&')
            .filter_map(|kv| {
                let mut it = kv.splitn(2, '=');
                Some((it.next()?, it.next()?))
            })
            .collect();
        let server = params.get("server")?.to_string();
        let port = params.get("port")?.to_string();
        let secret = params.get("secret")?.to_string();
        (secret, port, server)
    } else {
        return None;
    };
    let port: u16 = port_str.parse().ok()?;
    let secret_bytes = if query.len() >= 32 && query.chars().all(|c| c.is_ascii_hexdigit()) {
        (0..query.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&query[i..i + 2], 16).ok())
            .collect::<Vec<u8>>()
    } else {
        use base64::{
            Engine as _, engine::general_purpose::STANDARD_NO_PAD,
            engine::general_purpose::URL_SAFE_NO_PAD,
        };
        let padded = query.replace('-', "+").replace('_', "/");
        STANDARD_NO_PAD
            .decode(&padded)
            .or_else(|_| URL_SAFE_NO_PAD.decode(&query))
            .ok()?
    };
    let transport = match secret_bytes.first() {
        Some(&0xDD) => {
            let mut key = [0u8; 16];
            if let Some(slice) = secret_bytes.get(1..17) {
                key.copy_from_slice(slice);
            }
            TransportKind::PaddedIntermediate { secret: Some(key) }
        }
        Some(&0xEE) => {
            let mut key = [0u8; 16];
            if let Some(slice) = secret_bytes.get(1..17) {
                key.copy_from_slice(slice);
            }
            let domain = if secret_bytes.len() > 17 {
                String::from_utf8_lossy(&secret_bytes[17..]).into_owned()
            } else {
                host.clone()
            };
            TransportKind::FakeTls {
                secret: key,
                domain,
            }
        }
        _ => {
            let mut key = [0u8; 16];
            if let Some(slice) = secret_bytes.get(0..16) {
                key.copy_from_slice(slice);
            }
            TransportKind::Obfuscated {
                secret: if key == [0u8; 16] { None } else { Some(key) },
            }
        }
    };
    Some(MtProxyConfig {
        host,
        port,
        secret: secret_bytes,
        transport,
    })
}
