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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

const VERSION_V1: u8 = 1;
const VERSION_V2: u8 = 2;

const AUTH_KEY_LEN: usize = 256;

#[derive(Debug, Clone)]
pub struct FullSession {
    pub dc_id: u8,
    pub ip: IpAddr,
    pub port: u16,
    pub auth_key: [u8; AUTH_KEY_LEN],
    pub user_id: i64,
    pub server_salt: i64,
    pub seq_no: u32,
    pub layer: u32,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub dc_id: u8,
    pub ip: IpAddr,
    pub port: u16,
    pub auth_key: [u8; AUTH_KEY_LEN],
    pub user_id: i64,
}

#[derive(Debug, Clone)]
pub enum StringSession {
    V1(FullSession),
    V2(Session),
}

#[derive(Debug, thiserror::Error)]
pub enum StringSessionError {
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid or truncated session data")]
    InvalidData,
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("unknown ip type byte: {0}")]
    UnknownIpType(u8),
}

impl StringSession {
    /// Decode a string session. Auto-detects V1 or V2 from the version byte.
    pub fn decode(s: &str) -> Result<Self, StringSessionError> {
        let bytes = URL_SAFE_NO_PAD.decode(s.trim())?;

        if bytes.is_empty() {
            return Err(StringSessionError::InvalidData);
        }

        match bytes[0] {
            VERSION_V1 => decode_v1(&bytes).map(StringSession::V1),
            VERSION_V2 => decode_v2(&bytes).map(StringSession::V2),
            v => Err(StringSessionError::UnsupportedVersion(v)),
        }
    }

    /// Encode as V2 (minimal). This is the default.
    pub fn encode(&self) -> String {
        match self {
            StringSession::V2(s) => encode_v2(s),
            StringSession::V1(s) => encode_v2(&Session {
                dc_id: s.dc_id,
                ip: s.ip,
                port: s.port,
                auth_key: s.auth_key,
                user_id: s.user_id,
            }),
        }
    }

    /// Encode as V1 (full session with salt, seq_no, layer).
    /// Use this for manual transfer or when full state is needed.
    pub fn encode_v1(&self) -> String {
        match self {
            StringSession::V1(s) => encode_v1(s),
            StringSession::V2(_) => {
                panic!("cannot encode V2 session as V1: missing server_salt, seq_no, layer")
            }
        }
    }

    pub fn session(&self) -> Session {
        match self {
            StringSession::V2(s) => s.clone(),
            StringSession::V1(s) => Session {
                dc_id: s.dc_id,
                ip: s.ip,
                port: s.port,
                auth_key: s.auth_key,
                user_id: s.user_id,
            },
        }
    }

    pub fn full_session(&self) -> Option<&FullSession> {
        match self {
            StringSession::V1(s) => Some(s),
            StringSession::V2(_) => None,
        }
    }

    pub fn version(&self) -> u8 {
        match self {
            StringSession::V1(_) => VERSION_V1,
            StringSession::V2(_) => VERSION_V2,
        }
    }
}

impl From<Session> for StringSession {
    fn from(s: Session) -> Self {
        StringSession::V2(s)
    }
}

impl From<FullSession> for StringSession {
    fn from(s: FullSession) -> Self {
        StringSession::V1(s)
    }
}

fn encode_v2(s: &Session) -> String {
    let ip_bytes = ip_to_bytes(s.ip);
    let ip_type = ip_type_byte(s.ip);

    let mut buf = Vec::with_capacity(1 + 1 + 1 + ip_bytes.len() + 2 + 8 + AUTH_KEY_LEN);
    buf.push(VERSION_V2);
    buf.push(s.dc_id);
    buf.push(ip_type);
    buf.extend_from_slice(&ip_bytes);
    buf.extend_from_slice(&s.port.to_be_bytes());
    buf.extend_from_slice(&s.user_id.to_be_bytes());
    buf.extend_from_slice(&s.auth_key);

    URL_SAFE_NO_PAD.encode(&buf)
}

fn encode_v1(s: &FullSession) -> String {
    let ip_bytes = ip_to_bytes(s.ip);
    let ip_type = ip_type_byte(s.ip);

    let mut buf = Vec::with_capacity(1 + 1 + 1 + ip_bytes.len() + 2 + 8 + 8 + 4 + 4 + AUTH_KEY_LEN);
    buf.push(VERSION_V1);
    buf.push(s.dc_id);
    buf.push(ip_type);
    buf.extend_from_slice(&ip_bytes);
    buf.extend_from_slice(&s.port.to_be_bytes());
    buf.extend_from_slice(&s.user_id.to_be_bytes());
    buf.extend_from_slice(&s.server_salt.to_be_bytes());
    buf.extend_from_slice(&s.seq_no.to_be_bytes());
    buf.extend_from_slice(&s.layer.to_be_bytes());
    buf.extend_from_slice(&s.auth_key);

    URL_SAFE_NO_PAD.encode(&buf)
}

fn decode_v2(bytes: &[u8]) -> Result<Session, StringSessionError> {
    let mut c = 1usize;

    let dc_id = read_u8(bytes, &mut c)?;
    let ip = read_ip(bytes, &mut c)?;

    if bytes.len() < c + 2 + 8 + AUTH_KEY_LEN {
        return Err(StringSessionError::InvalidData);
    }

    let port = read_u16_be(bytes, &mut c)?;
    let user_id = read_i64_be(bytes, &mut c)?;
    let auth_key = read_auth_key(bytes, &mut c)?;

    Ok(Session {
        dc_id,
        ip,
        port,
        auth_key,
        user_id,
    })
}

fn decode_v1(bytes: &[u8]) -> Result<FullSession, StringSessionError> {
    let mut c = 1usize;

    let dc_id = read_u8(bytes, &mut c)?;
    let ip = read_ip(bytes, &mut c)?;

    if bytes.len() < c + 2 + 8 + 8 + 4 + 4 + AUTH_KEY_LEN {
        return Err(StringSessionError::InvalidData);
    }

    let port = read_u16_be(bytes, &mut c)?;
    let user_id = read_i64_be(bytes, &mut c)?;
    let server_salt = read_i64_be(bytes, &mut c)?;
    let seq_no = read_u32_be(bytes, &mut c)?;
    let layer = read_u32_be(bytes, &mut c)?;
    let auth_key = read_auth_key(bytes, &mut c)?;

    Ok(FullSession {
        dc_id,
        ip,
        port,
        auth_key,
        user_id,
        server_salt,
        seq_no,
        layer,
    })
}

fn read_u8(bytes: &[u8], c: &mut usize) -> Result<u8, StringSessionError> {
    if bytes.len() < *c + 1 {
        return Err(StringSessionError::InvalidData);
    }
    let v = bytes[*c];
    *c += 1;
    Ok(v)
}

fn read_u16_be(bytes: &[u8], c: &mut usize) -> Result<u16, StringSessionError> {
    let v = u16::from_be_bytes(
        bytes[*c..*c + 2]
            .try_into()
            .map_err(|_| StringSessionError::InvalidData)?,
    );
    *c += 2;
    Ok(v)
}

fn read_u32_be(bytes: &[u8], c: &mut usize) -> Result<u32, StringSessionError> {
    let v = u32::from_be_bytes(
        bytes[*c..*c + 4]
            .try_into()
            .map_err(|_| StringSessionError::InvalidData)?,
    );
    *c += 4;
    Ok(v)
}

fn read_i64_be(bytes: &[u8], c: &mut usize) -> Result<i64, StringSessionError> {
    let v = i64::from_be_bytes(
        bytes[*c..*c + 8]
            .try_into()
            .map_err(|_| StringSessionError::InvalidData)?,
    );
    *c += 8;
    Ok(v)
}

fn read_auth_key(bytes: &[u8], c: &mut usize) -> Result<[u8; AUTH_KEY_LEN], StringSessionError> {
    let key: [u8; AUTH_KEY_LEN] = bytes[*c..*c + AUTH_KEY_LEN]
        .try_into()
        .map_err(|_| StringSessionError::InvalidData)?;
    *c += AUTH_KEY_LEN;
    Ok(key)
}

fn read_ip(bytes: &[u8], c: &mut usize) -> Result<IpAddr, StringSessionError> {
    let ip_type = read_u8(bytes, c)?;
    match ip_type {
        4 => {
            if bytes.len() < *c + 4 {
                return Err(StringSessionError::InvalidData);
            }
            let octets: [u8; 4] = bytes[*c..*c + 4]
                .try_into()
                .map_err(|_| StringSessionError::InvalidData)?;
            *c += 4;
            Ok(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        6 => {
            if bytes.len() < *c + 16 {
                return Err(StringSessionError::InvalidData);
            }
            let octets: [u8; 16] = bytes[*c..*c + 16]
                .try_into()
                .map_err(|_| StringSessionError::InvalidData)?;
            *c += 16;
            Ok(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        other => Err(StringSessionError::UnknownIpType(other)),
    }
}

fn ip_to_bytes(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => v6.octets().to_vec(),
    }
}

fn ip_type_byte(ip: IpAddr) -> u8 {
    match ip {
        IpAddr::V4(_) => 4,
        IpAddr::V6(_) => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_key() -> [u8; AUTH_KEY_LEN] {
        let mut k = [0u8; AUTH_KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    fn ipv4() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(149, 154, 167, 51))
    }

    fn ipv6() -> IpAddr {
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xb28, 0xf23d, 0, 0, 0, 0, 0xa))
    }

    #[test]
    fn v2_roundtrip_ipv4() {
        let s = StringSession::V2(Session {
            dc_id: 2,
            ip: ipv4(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 123456789,
        });

        let encoded = s.encode();
        let decoded = StringSession::decode(&encoded).unwrap();

        assert_eq!(decoded.version(), 2);
        let d = decoded.session();
        assert_eq!(d.dc_id, 2);
        assert_eq!(d.ip, ipv4());
        assert_eq!(d.port, 443);
        assert_eq!(d.user_id, 123456789);
        assert_eq!(d.auth_key, dummy_key());
    }

    #[test]
    fn v2_roundtrip_ipv6() {
        let s = StringSession::V2(Session {
            dc_id: 4,
            ip: ipv6(),
            port: 443,
            auth_key: dummy_key(),
            user_id: -987654321,
        });

        let encoded = s.encode();
        let decoded = StringSession::decode(&encoded).unwrap();

        assert_eq!(decoded.version(), 2);
        let d = decoded.session();
        assert_eq!(d.ip, ipv6());
        assert_eq!(d.user_id, -987654321);
    }

    #[test]
    fn v1_roundtrip_ipv4() {
        let s = StringSession::V1(FullSession {
            dc_id: 1,
            ip: ipv4(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 111,
            server_salt: -999,
            seq_no: 42,
            layer: 166,
        });

        let encoded = s.encode_v1();
        let decoded = StringSession::decode(&encoded).unwrap();

        assert_eq!(decoded.version(), 1);
        let f = decoded.full_session().unwrap();
        assert_eq!(f.dc_id, 1);
        assert_eq!(f.ip, ipv4());
        assert_eq!(f.port, 443);
        assert_eq!(f.user_id, 111);
        assert_eq!(f.server_salt, -999);
        assert_eq!(f.seq_no, 42);
        assert_eq!(f.layer, 166);
        assert_eq!(f.auth_key, dummy_key());
    }

    #[test]
    fn v1_roundtrip_ipv6() {
        let s = StringSession::V1(FullSession {
            dc_id: 5,
            ip: ipv6(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 777,
            server_salt: 12345,
            seq_no: 10,
            layer: 166,
        });

        let encoded = s.encode_v1();
        let decoded = StringSession::decode(&encoded).unwrap();

        assert_eq!(decoded.version(), 1);
        let f = decoded.full_session().unwrap();
        assert_eq!(f.ip, ipv6());
        assert_eq!(f.layer, 166);
    }

    #[test]
    fn v1_encode_produces_v2_when_called_via_encode() {
        let s = StringSession::V1(FullSession {
            dc_id: 2,
            ip: ipv4(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 555,
            server_salt: 0,
            seq_no: 0,
            layer: 166,
        });

        let encoded = s.encode();
        let decoded = StringSession::decode(&encoded).unwrap();
        assert_eq!(decoded.version(), 2);
    }

    #[test]
    fn v2_encoded_length_ipv4() {
        let s = StringSession::V2(Session {
            dc_id: 1,
            ip: ipv4(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 1,
        });
        assert_eq!(s.encode().len(), 364);
    }

    #[test]
    fn rejects_truncated() {
        assert!(StringSession::decode("Ag").is_err());
    }

    #[test]
    fn rejects_unsupported_version() {
        let bad = URL_SAFE_NO_PAD.encode(&[99u8]);
        assert!(matches!(
            StringSession::decode(&bad),
            Err(StringSessionError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn full_session_returns_none_for_v2() {
        let s = StringSession::V2(Session {
            dc_id: 1,
            ip: ipv4(),
            port: 443,
            auth_key: dummy_key(),
            user_id: 1,
        });
        assert!(s.full_session().is_none());
    }
}
