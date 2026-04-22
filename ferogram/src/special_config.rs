// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Firebase / Google special-config fallback.
//!
//! When **both** normal TCP and DNS-over-HTTPS fail (heavily censored networks),
//! Telegram fetches DC configuration from three Google cloud services that are
//! harder to block wholesale than Telegram's own servers.
//!
//!
//! # Sources tried (in parallel, first-wins)
//!
//! | # | Service | URL pattern |
//! |---|---------|-------------|
//! | 1 | Firebase RemoteConfig | `https://firebaseremoteconfig.googleapis.com/v1/…` |
//! | 2 | Google Firestore | `https://firestore.googleapis.com/v1/…` |
//! | 3 | Firebase Realtime DB | `https://<project>.firebaseio.com/…` |
//!
//! The response from each source is a **base64-encoded, AES-IGE-encrypted**
//! blob.  After decryption a TL-serialised [`help.configSimple`] is returned,
//! which contains one or more DC addresses with phone-prefix rules.
//!
//! # Usage
//!
//! ```rust,no_run
//! use layer_client::special_config::SpecialConfig;
//!
//! #[tokio::main]
//! async fn main() {
//!     let cfg = SpecialConfig::new();
//!     if let Some(dcs) = cfg.fetch().await {
//!         for dc in dcs {
//!             println!("DC{}: {}:{}", dc.dc_id, dc.ip, dc.port);
//!         }
//!     }
//! }
//! ```

use base64::Engine as _;

// Hardcoded Firebase / Google API constants

/// Google Cloud project hosting RemoteConfig + Realtime DB.
const REMOTE_PROJECT: &str = "peak-vista-421";
/// Firestore project ID.
const FIRE_PROJECT: &str = "reserve-5a846";
/// Config key stored in Firebase services.
const CONFIG_KEY: &str = "ipconfig";
/// Config sub-key / version.
const CONFIG_SUB_KEY: &str = "v3";
/// Firebase / Google APIs API key (public, embedded in all Telegram clients).
const API_KEY: &str = "AIzaSyC2-kAkpDsroixRXw-sTw-Wfqo4NxjMwwM";
/// Firebase App ID for RemoteConfig requests.
const APP_ID: &str = "1:560508485281:web:4ee13a6af4e84d49e67ae0";

// RSA public key for verifying / deriving the decrypt key
//
// This is **not** the auth-key RSA key - it is a separate key published by
// Published by Telegram.
//
// Modulus (n) in big-endian hex, 256 bytes = 2048 bits.

/// Telegram's special-config RSA-2048 public key modulus (big-endian, 256 bytes).
///
/// Public key for decrypting special config responses.
#[rustfmt::skip]
const CONFIG_RSA_N: &[u8; 256] = &[
    0xca,0xbf,0xb5,0xf1,0x17,0xb1,0xda,0x88,0x6d,0x57,0x2f,0x2c,0xae,0x81,0x8f,0x07,
    0x05,0xc3,0xdc,0x33,0xa8,0x28,0x24,0xa9,0x8c,0x3a,0x98,0xa1,0x78,0x02,0xa8,0x1e,
    0xe2,0xa2,0x59,0xf8,0x78,0x30,0x85,0x7c,0xe0,0x54,0x95,0xf5,0xd4,0x12,0xf3,0x3f,
    0x7e,0x72,0x82,0xa4,0x5e,0x3a,0x56,0x40,0x1f,0xb6,0x56,0xf8,0x56,0xe3,0xc3,0x79,
    0x04,0x92,0xfd,0x9c,0x59,0x60,0x41,0xaa,0x1d,0xac,0xb0,0x96,0xba,0x15,0x9d,0x71,
    0xc8,0x8e,0x0a,0xa8,0xc6,0x20,0x1d,0xd7,0xdd,0xb1,0x44,0x6a,0xde,0xb9,0x72,0x1a,
    0x50,0xa9,0xa4,0xc2,0x53,0x3d,0x24,0x80,0xfd,0x59,0x2d,0xa3,0x52,0xc4,0xe9,0xcd,
    0x0f,0x75,0x2f,0xc3,0x04,0x3c,0xb2,0x7f,0xe3,0x3a,0xc1,0xc1,0x9b,0x9a,0xf1,0x6e,
    0x5f,0x10,0x2c,0x02,0x0a,0x1d,0x9c,0xf4,0x6e,0x48,0xcf,0x30,0x66,0xfa,0x8b,0x4c,
    0x4b,0xf6,0x0a,0xc2,0x64,0x75,0xa2,0x5c,0xd1,0x0b,0x21,0x24,0xc8,0x01,0x23,0x5d,
    0x6a,0x81,0x23,0xd1,0x6d,0xbf,0x97,0x86,0xf2,0x6d,0x15,0x90,0x1c,0xce,0x1b,0xae,
    0x79,0x58,0x86,0x1d,0xc9,0x5d,0x07,0x7c,0x32,0xbf,0x35,0x67,0x2f,0x1a,0xa6,0xb4,
    0xc3,0xf9,0xeb,0x88,0xc0,0xfa,0x98,0x38,0xca,0x3a,0xbc,0x9a,0x9b,0x0d,0x0e,0x3f,
    0xd4,0x2e,0x62,0x03,0x0d,0xd0,0x2f,0x71,0x31,0x6f,0x72,0xf1,0x29,0x8f,0xe2,0x1f,
    0x76,0x38,0x05,0xa8,0x75,0x36,0x20,0x63,0x58,0x59,0x0b,0x17,0x6f,0xe9,0xcb,0x39,
    0x52,0x75,0xbd,0xf8,0xf5,0xaf,0x4b,0xe5,0x88,0x6e,0x48,0x7e,0x19,0xa5,0x98,0xc7,
];

// Public types

/// One DC address entry decoded from the special config.
#[derive(Debug, Clone)]
pub struct ConfigDcOption {
    /// DC identifier (1-5).
    pub dc_id: i32,
    /// IPv4 or IPv6 address string.
    pub ip: String,
    /// Port number.
    pub port: u16,
}

/// Errors that can occur during special config fetching / decryption.
#[derive(Debug)]
pub enum SpecialConfigError {
    Http(String),
    Decode(String),
    Decrypt(String),
    Parse(String),
}

impl std::fmt::Display for SpecialConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP: {e}"),
            Self::Decode(e) => write!(f, "base64/JSON: {e}"),
            Self::Decrypt(e) => write!(f, "decrypt: {e}"),
            Self::Parse(e) => write!(f, "TL parse: {e}"),
        }
    }
}

impl std::error::Error for SpecialConfigError {}

/// Client for fetching Telegram DC addresses from Firebase/Google fallback
/// services when normal TCP and DoH are both unreachable.
#[derive(Clone)]
pub struct SpecialConfig {
    client: reqwest::Client,
}

impl std::fmt::Debug for SpecialConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpecialConfig").finish_non_exhaustive()
    }
}

impl SpecialConfig {
    /// Create a new `SpecialConfig` client.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .no_proxy()
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                 AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/124.0.0.0 Safari/537.36",
            )
            .build()
            .expect("SpecialConfig: failed to build reqwest client");
        Self { client }
    }

    // Public API

    /// Fetch DC options from all three Firebase sources in parallel.
    ///
    /// Returns `None` only if every source fails.
    /// The first successful decryption result is returned; others are cancelled.
    pub async fn fetch(&self) -> Option<Vec<ConfigDcOption>> {
        let (r1, r2, r3) = tokio::join!(
            self.fetch_remote_config(),
            self.fetch_firestore(),
            self.fetch_realtime_db(),
        );

        for result in [r1, r2, r3] {
            match result {
                Ok(dcs) if !dcs.is_empty() => return Some(dcs),
                Ok(_) => {}
                Err(e) => tracing::debug!("[special_config] source failed: {e}"),
            }
        }
        tracing::warn!("[special_config] all sources failed");
        None
    }

    // Source #1 - Firebase Remote Config

    async fn fetch_remote_config(&self) -> Result<Vec<ConfigDcOption>, SpecialConfigError> {
        // POST https://firebaseremoteconfig.googleapis.com/v1/projects/<proj>/namespaces/firebase:fetch
        let url = format!(
            "https://firebaseremoteconfig.googleapis.com/v1/projects/{REMOTE_PROJECT}/namespaces/firebase:fetch?key={API_KEY}"
        );
        let instance_id = generate_instance_id();
        let body = serde_json::json!({
            "appId": APP_ID,
            "appInstanceId": instance_id,
        });

        tracing::debug!("[special_config] RemoteConfig → {url}");
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        let encoded = parse_remote_config_response(&bytes)?;
        let raw = base64_decode(&encoded)?;
        decrypt_and_parse(&raw)
    }

    // Source #2 - Google Firestore

    async fn fetch_firestore(&self) -> Result<Vec<ConfigDcOption>, SpecialConfigError> {
        // GET https://firestore.googleapis.com/v1/projects/<proj>/databases/(default)/documents/<key>/<sub>
        let url = format!(
            "https://firestore.googleapis.com/v1/projects/{FIRE_PROJECT}/databases/(default)/documents/{CONFIG_KEY}/{CONFIG_SUB_KEY}?key={API_KEY}"
        );

        tracing::debug!("[special_config] Firestore → {url}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        let encoded = parse_firestore_response(&bytes)?;
        let raw = base64_decode(&encoded)?;
        decrypt_and_parse(&raw)
    }

    // Source #3 - Firebase Realtime Database

    async fn fetch_realtime_db(&self) -> Result<Vec<ConfigDcOption>, SpecialConfigError> {
        // GET https://<project>.firebaseio.com/<key>/<subkey>.json
        let url = format!(
            "https://{REMOTE_PROJECT}-default-rtdb.firebaseio.com/{CONFIG_KEY}/{CONFIG_SUB_KEY}.json"
        );

        tracing::debug!("[special_config] RealtimeDB → {url}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SpecialConfigError::Http(e.to_string()))?;

        // Realtime DB returns the value directly as a JSON string.
        let encoded = parse_realtime_db_response(&bytes)?;
        let raw = base64_decode(&encoded)?;
        decrypt_and_parse(&raw)
    }
}

impl Default for SpecialConfig {
    fn default() -> Self {
        Self::new()
    }
}

// Response parsers

/// Extract the config payload from a Firebase RemoteConfig JSON response.
///
/// Shape: `{ "entries": { "ipconfigv3": "<base64>" } }`
fn parse_remote_config_response(bytes: &[u8]) -> Result<String, SpecialConfigError> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| SpecialConfigError::Decode(e.to_string()))?;
    let key = format!("{CONFIG_KEY}{CONFIG_SUB_KEY}");
    v.get("entries")
        .and_then(|e| e.get(&key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
        .ok_or_else(|| SpecialConfigError::Decode("entries key missing".into()))
}

/// Extract the config payload from a Firestore JSON response.
///
/// Shape: `{ "fields": { "ipconfigv3": { "stringValue": "<base64>" } } }`
fn parse_firestore_response(bytes: &[u8]) -> Result<String, SpecialConfigError> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| SpecialConfigError::Decode(e.to_string()))?;
    let key = format!("{CONFIG_KEY}{CONFIG_SUB_KEY}");
    v.get("fields")
        .and_then(|f| f.get(&key))
        .and_then(|fv| fv.get("stringValue"))
        .and_then(|sv| sv.as_str())
        .map(|s| s.to_owned())
        .ok_or_else(|| SpecialConfigError::Decode("fields/stringValue missing".into()))
}

/// Extract the config payload from a Firebase Realtime DB JSON response.
///
/// The value is stored directly as a JSON string: `"<base64>"`.
fn parse_realtime_db_response(bytes: &[u8]) -> Result<String, SpecialConfigError> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| SpecialConfigError::Decode(e.to_string()))?;
    v.as_str()
        .map(|s| s.to_owned())
        .ok_or_else(|| SpecialConfigError::Decode("expected JSON string at root".into()))
}

// Decryption + TL parsing

/// Decrypt the raw bytes and parse the embedded DC options.
///
/// # Wire format
///
/// The special config blob has the following layout after base64 decoding:
///
/// ```text
/// [ 4 bytes: AES-IGE key seed (random, discarded after key derivation) ]
/// [ N bytes: AES-IGE ciphertext of TL-encoded help.configSimple        ]
/// ```
///
/// The AES-256-IGE key and IV are derived as:
///
/// ```text
/// key = SHA-256( CONFIG_RSA_N || blob[0..4] )[0..32]
/// iv  = SHA-256( blob[0..4] || CONFIG_RSA_N )[0..32]
/// ```
///
/// The decrypted plaintext starts with a 32-byte SHA-256 integrity check,
/// followed by the TL body of `help.configSimple`.
fn decrypt_and_parse(blob: &[u8]) -> Result<Vec<ConfigDcOption>, SpecialConfigError> {
    if blob.len() < 36 {
        return Err(SpecialConfigError::Decrypt(format!(
            "blob too short: {} bytes",
            blob.len()
        )));
    }

    // Key derivation:
    // key = SHA256(n || seed)  iv = SHA256(seed || n)
    let seed = &blob[..4];
    let key_src: Vec<u8> = CONFIG_RSA_N.iter().chain(seed.iter()).copied().collect();
    let iv_src: Vec<u8> = seed.iter().chain(CONFIG_RSA_N.iter()).copied().collect();

    let key: [u8; 32] = sha2_256(&key_src)[..32].try_into().unwrap();
    let iv: [u8; 32] = sha2_256(&iv_src)[..32].try_into().unwrap();

    let mut ciphertext = blob[4..].to_vec();
    // Pad to 16-byte block boundary (AES-IGE requirement).
    let rem = ciphertext.len() % 16;
    if rem != 0 {
        ciphertext.extend(std::iter::repeat_n(0u8, 16 - rem));
    }

    ferogram_crypto::aes::ige_decrypt(&mut ciphertext, &key, &iv);

    // First 32 bytes = SHA-256 integrity check of the rest.
    if ciphertext.len() < 32 {
        return Err(SpecialConfigError::Decrypt(
            "decrypted blob too short".into(),
        ));
    }
    let expected_hash: [u8; 32] = ciphertext[..32].try_into().unwrap();
    let actual_hash = sha2_256(&ciphertext[32..]);
    if expected_hash != actual_hash {
        return Err(SpecialConfigError::Decrypt(
            "integrity check failed - wrong key or corrupted blob".into(),
        ));
    }

    parse_config_simple(&ciphertext[32..])
}

/// Parse `help.configSimple` TL.
///
/// ```text
/// help.configSimple#5a592a6c
///     date:int expires:int
///     dc_options:Vector<AccessPointRule>
///   = help.ConfigSimple;
///
/// accessPointRule#4679b65f
///     phone_prefix_rules:string
///     dc_id:int
///     ips:Vector<IpPort>
///   = AccessPointRule;
///
/// ipPort#d433ad73 ipv4:int port:int = IpPort;
/// ipPortSecret#37982646 ipv4:int port:int secret:bytes = IpPort;
/// ```
fn parse_config_simple(data: &[u8]) -> Result<Vec<ConfigDcOption>, SpecialConfigError> {
    let mut pos = 0;

    // Constructor: help.configSimple#5a592a6c
    let cid = read_u32(data, &mut pos)?;
    if cid != 0x5a592a6c {
        return Err(SpecialConfigError::Parse(format!(
            "unexpected constructor {cid:#010x} (expected help.configSimple)"
        )));
    }

    let _date = read_i32(data, &mut pos)?;
    let _expires = read_i32(data, &mut pos)?;

    // Vector<AccessPointRule>
    let vec_cid = read_u32(data, &mut pos)?;
    if vec_cid != 0x1cb5c415 {
        return Err(SpecialConfigError::Parse(format!(
            "expected Vector constructor, got {vec_cid:#010x}"
        )));
    }
    let rule_count = read_u32(data, &mut pos)? as usize;

    let mut options = Vec::new();

    for _ in 0..rule_count {
        let rule_cid = read_u32(data, &mut pos)?;
        if rule_cid != 0x4679b65f {
            return Err(SpecialConfigError::Parse(format!(
                "expected accessPointRule, got {rule_cid:#010x}"
            )));
        }
        let _prefix = read_tl_string(data, &mut pos)?; // phone_prefix_rules
        let dc_id = read_i32(data, &mut pos)?;

        // Vector<IpPort>
        let ip_vec_cid = read_u32(data, &mut pos)?;
        if ip_vec_cid != 0x1cb5c415 {
            return Err(SpecialConfigError::Parse(format!(
                "expected Vector for IpPort, got {ip_vec_cid:#010x}"
            )));
        }
        let ip_count = read_u32(data, &mut pos)? as usize;

        for _ in 0..ip_count {
            let ip_cid = read_u32(data, &mut pos)?;
            match ip_cid {
                // ipPort#d433ad73 ipv4:int port:int
                0xd433ad73 => {
                    let ipv4_raw = read_u32(data, &mut pos)? as u32;
                    let port = read_u32(data, &mut pos)? as u16;
                    let ip = format!(
                        "{}.{}.{}.{}",
                        (ipv4_raw >> 24) & 0xff,
                        (ipv4_raw >> 16) & 0xff,
                        (ipv4_raw >> 8) & 0xff,
                        ipv4_raw & 0xff,
                    );
                    options.push(ConfigDcOption { dc_id, ip, port });
                }
                // ipPortSecret#37982646 ipv4:int port:int secret:bytes
                0x37982646 => {
                    let ipv4_raw = read_u32(data, &mut pos)? as u32;
                    let port = read_u32(data, &mut pos)? as u16;
                    let _secret = read_tl_bytes(data, &mut pos)?;
                    let ip = format!(
                        "{}.{}.{}.{}",
                        (ipv4_raw >> 24) & 0xff,
                        (ipv4_raw >> 16) & 0xff,
                        (ipv4_raw >> 8) & 0xff,
                        ipv4_raw & 0xff,
                    );
                    options.push(ConfigDcOption { dc_id, ip, port });
                }
                other => {
                    return Err(SpecialConfigError::Parse(format!(
                        "unknown IpPort constructor {other:#010x}"
                    )));
                }
            }
        }
    }

    tracing::debug!(
        "[special_config] decoded {} DC options from help.configSimple",
        options.len()
    );
    Ok(options)
}

// TL reading helpers

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, SpecialConfigError> {
    if *pos + 4 > data.len() {
        return Err(SpecialConfigError::Parse(format!(
            "unexpected EOF at pos {pos}"
        )));
    }
    let v = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

fn read_i32(data: &[u8], pos: &mut usize) -> Result<i32, SpecialConfigError> {
    read_u32(data, pos).map(|v| v as i32)
}

/// Read a TL `bytes` field (length-prefixed, 4-byte aligned).
fn read_tl_bytes(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, SpecialConfigError> {
    if *pos >= data.len() {
        return Err(SpecialConfigError::Parse("EOF reading bytes length".into()));
    }
    let (len, overhead) = if data[*pos] < 254 {
        (data[*pos] as usize, 1)
    } else if *pos + 4 <= data.len() {
        let l = data[*pos + 1] as usize
            | (data[*pos + 2] as usize) << 8
            | (data[*pos + 3] as usize) << 16;
        (l, 4)
    } else {
        return Err(SpecialConfigError::Parse(
            "EOF reading 3-byte bytes length".into(),
        ));
    };
    *pos += overhead;
    if *pos + len > data.len() {
        return Err(SpecialConfigError::Parse(format!(
            "bytes field overruns buffer: need {len}, have {}",
            data.len() - *pos
        )));
    }
    let bytes = data[*pos..*pos + len].to_vec();
    *pos += len;
    // Padding to 4-byte boundary.
    let total = overhead + len;
    let pad = (4 - total % 4) % 4;
    *pos += pad;
    Ok(bytes)
}

/// Read a TL `string` field (identical wire format to `bytes`).
fn read_tl_string(data: &[u8], pos: &mut usize) -> Result<String, SpecialConfigError> {
    let bytes = read_tl_bytes(data, pos)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// Utilities

fn base64_decode(s: &str) -> Result<Vec<u8>, SpecialConfigError> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| SpecialConfigError::Decode(e.to_string()))
}

fn sha2_256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(data);
    h.finalize().into()
}

/// Generate a random Firebase instance ID (22 base64url chars, first nibble 0x7).
///
fn generate_instance_id() -> String {
    let mut fid = [0u8; 17];
    getrandom::getrandom(&mut fid).expect("getrandom");
    fid[0] = (fid[0] & 0xF0) | 0x07;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(fid)[..22].to_owned()
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_config_parse() {
        let json = r#"{"entries":{"ipconfigv3":"SGVsbG8="}}"#;
        let result = parse_remote_config_response(json.as_bytes()).unwrap();
        assert_eq!(result, "SGVsbG8=");
    }

    #[test]
    fn firestore_parse() {
        let json = r#"{"fields":{"ipconfigv3":{"stringValue":"SGVsbG8="}}}"#;
        let result = parse_firestore_response(json.as_bytes()).unwrap();
        assert_eq!(result, "SGVsbG8=");
    }

    #[test]
    fn realtime_db_parse() {
        let json = r#""SGVsbG8=""#;
        let result = parse_realtime_db_response(json.as_bytes()).unwrap();
        assert_eq!(result, "SGVsbG8=");
    }

    #[test]
    fn base64_decode_ok() {
        let dec = base64_decode("SGVsbG8=").unwrap();
        assert_eq!(dec, b"Hello");
    }

    #[test]
    fn instance_id_length() {
        let id = generate_instance_id();
        assert_eq!(id.len(), 22, "instance ID must be 22 chars");
    }

    #[test]
    fn tl_bytes_roundtrip() {
        // Build a small TL bytes field manually and parse it back.
        let payload = b"testdata";
        let mut buf = vec![payload.len() as u8]; // 1-byte length
        buf.extend_from_slice(payload);
        // Pad to 4 bytes: 1 + 8 = 9 → pad 3
        buf.extend_from_slice(&[0, 0, 0]);

        let mut pos = 0;
        let got = read_tl_bytes(&buf, &mut pos).unwrap();
        assert_eq!(got, payload);
        assert_eq!(pos, buf.len());
    }
}
