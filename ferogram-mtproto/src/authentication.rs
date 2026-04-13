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

//! Sans-IO MTProto authorization key generation.
//!
//! # Flow
//!
//! ```text
//! let (req, s1) = authentication::step1()?;
//! // send req, receive resp (ResPQ)
//! let (req, s2) = authentication::step2(s1, resp, dc_id)?;
//! // send req, receive resp (ServerDhParams)
//! let (req, s3) = authentication::step3(s2, resp)?;
//! // send req, receive resp (SetClientDhParamsAnswer)
//! let result = authentication::finish(s3, resp)?;
//! // on FinishResult::Done(d): d.auth_key is ready
//! // on FinishResult::Retry{..}: call retry_step3() + finish() up to 5 times
//! ```

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use ferogram_crypto::{AuthKey, aes, check_p_and_g, factorize, generate_key_data_from_nonce, rsa};
use ferogram_tl_types::{Cursor, Deserializable, Serializable};
use num_bigint::BigUint;
use sha1::{Digest, Sha1};

// Manual TL serialization helper for PQInnerDataDc
//
// Constructor: p_q_inner_data_dc#a9f55f95
//   pq:string p:string q:string nonce:int128 server_nonce:int128 new_nonce:int256 dc:int
//
// TL "string" (bytes) encoding: if len < 254 -> [len_byte, data..., 0-pad to 4-align],
// else [0xfe, len_lo, len_mid, len_hi, data..., 0-pad to 4-align].
fn tl_serialize_bytes(v: &[u8]) -> Vec<u8> {
    let len = v.len();
    let mut out = Vec::new();
    if len < 254 {
        out.push(len as u8);
        out.extend_from_slice(v);
        let total = 1 + len;
        let pad = (4 - total % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    } else {
        out.push(0xfe);
        out.push((len & 0xff) as u8);
        out.push(((len >> 8) & 0xff) as u8);
        out.push(((len >> 16) & 0xff) as u8);
        out.extend_from_slice(v);
        let total = 4 + len;
        let pad = (4 - total % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out
}

/// Serialize a `p_q_inner_data_dc` (constructor 0xa9f55f95) from raw fields.
/// Needed because the generated TL bindings only expose `PQInnerData`
/// (legacy, no DC id) which Telegram rejects for non-DC2 connections.
fn serialize_pq_inner_data_dc(
    pq: &[u8],
    p: &[u8],
    q: &[u8],
    nonce: &[u8; 16],
    server_nonce: &[u8; 16],
    new_nonce: &[u8; 32],
    dc_id: i32,
) -> Vec<u8> {
    let mut out = Vec::new();
    // Constructor id (little-endian)
    out.extend_from_slice(&0xa9f55f95_u32.to_le_bytes());
    out.extend(tl_serialize_bytes(pq));
    out.extend(tl_serialize_bytes(p));
    out.extend(tl_serialize_bytes(q));
    out.extend_from_slice(nonce);
    out.extend_from_slice(server_nonce);
    out.extend_from_slice(new_nonce);
    out.extend_from_slice(&dc_id.to_le_bytes());
    out
}

/// Errors that can occur during auth key generation.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    InvalidNonce {
        got: [u8; 16],
        expected: [u8; 16],
    },
    InvalidPqSize {
        size: usize,
    },
    UnknownFingerprints {
        fingerprints: Vec<i64>,
    },
    DhParamsFail,
    InvalidServerNonce {
        got: [u8; 16],
        expected: [u8; 16],
    },
    EncryptedResponseNotPadded {
        len: usize,
    },
    InvalidDhInnerData {
        error: ferogram_tl_types::deserialize::Error,
    },
    InvalidDhPrime {
        source: ferogram_crypto::DhError,
    },
    GParameterOutOfRange {
        value: BigUint,
        low: BigUint,
        high: BigUint,
    },
    DhGenRetry,
    DhGenFail,
    InvalidAnswerHash {
        got: [u8; 20],
        expected: [u8; 20],
    },
    InvalidNewNonceHash {
        got: [u8; 16],
        expected: [u8; 16],
    },
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNonce { got, expected } => {
                write!(f, "nonce mismatch: got {got:?}, expected {expected:?}")
            }
            Self::InvalidPqSize { size } => write!(f, "pq size {size} invalid (expected 8)"),
            Self::UnknownFingerprints { fingerprints } => {
                write!(f, "no known fingerprint in {fingerprints:?}")
            }
            Self::DhParamsFail => write!(f, "server returned DH params failure"),
            Self::InvalidServerNonce { got, expected } => write!(
                f,
                "server_nonce mismatch: got {got:?}, expected {expected:?}"
            ),
            Self::EncryptedResponseNotPadded { len } => {
                write!(f, "encrypted answer len {len} is not 16-byte aligned")
            }
            Self::InvalidDhInnerData { error } => {
                write!(f, "DH inner data deserialization error: {error}")
            }
            Self::InvalidDhPrime { source } => {
                write!(f, "DH prime/generator validation failed: {source}")
            }
            Self::GParameterOutOfRange { value, low, high } => {
                write!(f, "g={value} not in range ({low}, {high})")
            }
            Self::DhGenRetry => write!(f, "DH gen retry requested"),
            Self::DhGenFail => write!(f, "DH gen failed"),
            Self::InvalidAnswerHash { got, expected } => write!(
                f,
                "answer hash mismatch: got {got:?}, expected {expected:?}"
            ),
            Self::InvalidNewNonceHash { got, expected } => write!(
                f,
                "new nonce hash mismatch: got {got:?}, expected {expected:?}"
            ),
        }
    }
}

/// State after step 1.
pub struct Step1 {
    nonce: [u8; 16],
}

/// State after step 2.
#[derive(Clone)]
pub struct Step2 {
    nonce: [u8; 16],
    server_nonce: [u8; 16],
    new_nonce: [u8; 32],
}

/// Pre-processed server DH parameters retained so that step 3 can be
/// repeated on `dh_gen_retry` without having to re-decrypt the server response.
#[derive(Clone)]
pub struct DhParamsForRetry {
    /// Server-supplied DH prime (big-endian bytes).
    pub dh_prime: Vec<u8>,
    /// DH generator `g`.
    pub g: u32,
    /// Server's public DH value `g_a` (big-endian bytes).
    pub g_a: Vec<u8>,
    /// Server's reported Unix timestamp (used to compute `time_offset`).
    pub server_time: i32,
    /// AES key derived from nonces for this session's IGE encryption.
    pub aes_key: [u8; 32],
    /// AES IV derived from nonces for this session's IGE encryption.
    pub aes_iv: [u8; 32],
}

/// State after step 3.
pub struct Step3 {
    nonce: [u8; 16],
    server_nonce: [u8; 16],
    new_nonce: [u8; 32],
    time_offset: i32,
    /// Auth key candidate bytes (needed to derive `auth_key_aux_hash` on retry).
    auth_key: [u8; 256],
    /// The processed DH parameters stored so `retry_step3` can re-derive g_b
    /// without re-parsing the encrypted server response.
    pub dh_params: DhParamsForRetry,
}

/// Result of [`finish`] either the handshake is done, or the server wants us
/// to retry step 3 with the `auth_key_aux_hash` as `retry_id`.
pub enum FinishResult {
    /// Handshake complete.
    Done(Finished),
    /// Server sent `dh_gen_retry`.  Call [`retry_step3`] with the returned
    /// `retry_id` and the stored [`DhParamsForRetry`] from the previous Step3.
    Retry {
        /// The `auth_key_aux_hash` to embed as `retry_id` in the next attempt.
        retry_id: i64,
        /// DH parameters to feed back into [`retry_step3`].
        dh_params: DhParamsForRetry,
        /// Client nonce from the original step 1.
        nonce: [u8; 16],
        /// Server nonce from the ResPQ response.
        server_nonce: [u8; 16],
        /// Fresh nonce generated in step 2.
        new_nonce: [u8; 32],
    },
}

/// The final output of a successful auth key handshake.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    /// The 256-byte Telegram authorization key.
    pub auth_key: [u8; 256],
    /// Clock skew in seconds relative to the server.
    pub time_offset: i32,
    /// Initial server salt.
    pub first_salt: i64,
}

/// Generate a `req_pq_multi` request. Returns the request + opaque state.
pub fn step1() -> Result<(ferogram_tl_types::functions::ReqPqMulti, Step1), Error> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("getrandom");
    do_step1(&buf)
}

fn do_step1(random: &[u8; 16]) -> Result<(ferogram_tl_types::functions::ReqPqMulti, Step1), Error> {
    let nonce = *random;
    Ok((
        ferogram_tl_types::functions::ReqPqMulti { nonce },
        Step1 { nonce },
    ))
}

/// Process `ResPQ` and generate `req_DH_params`.
///
/// `dc_id` must be the numerical DC id of the server we are connecting to
/// (e.g. 1 ... 5).  It is embedded in the `PQInnerDataDc` payload so that
/// Telegram can reject misrouted handshakes on non-DC2 endpoints.
pub fn step2(
    data: Step1,
    response: ferogram_tl_types::enums::ResPq,
    dc_id: i32,
) -> Result<(ferogram_tl_types::functions::ReqDhParams, Step2), Error> {
    let mut rnd = [0u8; 256];
    getrandom::getrandom(&mut rnd).expect("getrandom");
    do_step2(data, response, &rnd, dc_id)
}

fn do_step2(
    data: Step1,
    response: ferogram_tl_types::enums::ResPq,
    random: &[u8; 256],
    dc_id: i32,
) -> Result<(ferogram_tl_types::functions::ReqDhParams, Step2), Error> {
    let Step1 { nonce } = data;

    // ResPq has a single constructor: resPQ -> variant ResPq
    let ferogram_tl_types::enums::ResPq::ResPq(res_pq) = response;

    check_nonce(&res_pq.nonce, &nonce)?;

    if res_pq.pq.len() != 8 {
        return Err(Error::InvalidPqSize {
            size: res_pq.pq.len(),
        });
    }

    let pq = u64::from_be_bytes(res_pq.pq.as_slice().try_into().unwrap());
    let (p, q) = factorize(pq);

    let mut new_nonce = [0u8; 32];
    new_nonce.copy_from_slice(&random[..32]);

    // random[32..256] is 224 bytes for RSA padding
    let rnd224: &[u8; 224] = random[32..].try_into().unwrap();

    fn trim_be(v: u64) -> Vec<u8> {
        let b = v.to_be_bytes();
        let skip = b.iter().position(|&x| x != 0).unwrap_or(7);
        b[skip..].to_vec()
    }

    let p_bytes = trim_be(p);
    let q_bytes = trim_be(q);

    // Serialize PQInnerDataDc (constructor 0xa9f55f95) manually.
    // The legacy PQInnerData constructor (#83c95aec, no dc field) is rejected
    // by Telegram servers for connections to non-DC2 endpoints.
    let pq_inner = serialize_pq_inner_data_dc(
        &pq.to_be_bytes(),
        &p_bytes,
        &q_bytes,
        &nonce,
        &res_pq.server_nonce,
        &new_nonce,
        dc_id,
    );

    let fingerprint = res_pq
        .server_public_key_fingerprints
        .iter()
        .copied()
        .find(|&fp| key_for_fingerprint(fp).is_some())
        .ok_or_else(|| Error::UnknownFingerprints {
            fingerprints: res_pq.server_public_key_fingerprints.clone(),
        })?;

    let key = key_for_fingerprint(fingerprint).unwrap();
    let ciphertext = rsa::encrypt_hashed(&pq_inner, &key, rnd224);

    Ok((
        ferogram_tl_types::functions::ReqDhParams {
            nonce,
            server_nonce: res_pq.server_nonce,
            p: p_bytes,
            q: q_bytes,
            public_key_fingerprint: fingerprint,
            encrypted_data: ciphertext,
        },
        Step2 {
            nonce,
            server_nonce: res_pq.server_nonce,
            new_nonce,
        },
    ))
}

/// Process `ServerDhParams` into a reusable [`DhParamsForRetry`] + send the
/// first `set_client_DH_params` request.
///
/// `retry_id` should be 0 on the first call, or `auth_key_aux_hash` (returned
/// by [`finish`] as [`FinishResult::Retry`]) on subsequent attempts.
pub fn step3(
    data: Step2,
    response: ferogram_tl_types::enums::ServerDhParams,
) -> Result<(ferogram_tl_types::functions::SetClientDhParams, Step3), Error> {
    let mut rnd = [0u8; 272];
    getrandom::getrandom(&mut rnd).expect("getrandom");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i32;
    do_step3(data, response, &rnd, now, 0)
}

/// Re-run the client DH params generation after a `dh_gen_retry` response.
/// Feed the `dh_params`, `nonce`, `server_nonce`, `new_nonce` from
/// [`FinishResult::Retry`] and the `retry_id` (= `auth_key_aux_hash`).
pub fn retry_step3(
    dh_params: &DhParamsForRetry,
    nonce: [u8; 16],
    server_nonce: [u8; 16],
    new_nonce: [u8; 32],
    retry_id: i64,
) -> Result<(ferogram_tl_types::functions::SetClientDhParams, Step3), Error> {
    let mut rnd = [0u8; 272];
    getrandom::getrandom(&mut rnd).expect("getrandom");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i32;
    generate_client_dh_params(
        dh_params,
        nonce,
        server_nonce,
        new_nonce,
        retry_id,
        &rnd,
        now,
    )
}

fn generate_client_dh_params(
    dh: &DhParamsForRetry,
    nonce: [u8; 16],
    server_nonce: [u8; 16],
    new_nonce: [u8; 32],
    retry_id: i64,
    random: &[u8; 272],
    now: i32,
) -> Result<(ferogram_tl_types::functions::SetClientDhParams, Step3), Error> {
    let dh_prime = BigUint::from_bytes_be(&dh.dh_prime);
    let g = BigUint::from(dh.g);
    let g_a = BigUint::from_bytes_be(&dh.g_a);
    let time_offset = dh.server_time - now;

    let b = BigUint::from_bytes_be(&random[..256]);
    let g_b = g.modpow(&b, &dh_prime);

    let one = BigUint::from(1u32);
    let safety = one.clone() << (2048 - 64);
    check_g_in_range(&g_b, &one, &(&dh_prime - &one))?;
    check_g_in_range(&g_b, &safety, &(&dh_prime - &safety))?;

    let client_dh_inner = ferogram_tl_types::enums::ClientDhInnerData::ClientDhInnerData(
        ferogram_tl_types::types::ClientDhInnerData {
            nonce,
            server_nonce,
            retry_id,
            g_b: g_b.to_bytes_be(),
        },
    )
    .to_bytes();

    let digest: [u8; 20] = {
        let mut sha = Sha1::new();
        sha.update(&client_dh_inner);
        sha.finalize().into()
    };

    let pad_len = (16 - ((20 + client_dh_inner.len()) % 16)) % 16;
    let rnd16 = &random[256..256 + pad_len.min(16)];

    let mut hashed = Vec::with_capacity(20 + client_dh_inner.len() + pad_len);
    hashed.extend_from_slice(&digest);
    hashed.extend_from_slice(&client_dh_inner);
    hashed.extend_from_slice(&rnd16[..pad_len]);

    let key: [u8; 32] = dh.aes_key;
    let iv: [u8; 32] = dh.aes_iv;
    aes::ige_encrypt(&mut hashed, &key, &iv);

    // Compute auth_key = g_a^b mod dh_prime for this attempt.
    let mut auth_key_bytes = [0u8; 256];
    let gab_bytes = g_a.modpow(&b, &dh_prime).to_bytes_be();
    let skip = 256 - gab_bytes.len();
    auth_key_bytes[skip..].copy_from_slice(&gab_bytes);

    Ok((
        ferogram_tl_types::functions::SetClientDhParams {
            nonce,
            server_nonce,
            encrypted_data: hashed,
        },
        Step3 {
            nonce,
            server_nonce,
            new_nonce,
            time_offset,
            auth_key: auth_key_bytes,
            dh_params: dh.clone(),
        },
    ))
}

fn do_step3(
    data: Step2,
    response: ferogram_tl_types::enums::ServerDhParams,
    random: &[u8; 272],
    now: i32,
    retry_id: i64,
) -> Result<(ferogram_tl_types::functions::SetClientDhParams, Step3), Error> {
    let Step2 {
        nonce,
        server_nonce,
        new_nonce,
    } = data;

    let mut server_dh_ok = match response {
        ferogram_tl_types::enums::ServerDhParams::Fail(f) => {
            check_nonce(&f.nonce, &nonce)?;
            check_server_nonce(&f.server_nonce, &server_nonce)?;
            return Err(Error::DhParamsFail);
        }
        ferogram_tl_types::enums::ServerDhParams::Ok(x) => x,
    };

    check_nonce(&server_dh_ok.nonce, &nonce)?;
    check_server_nonce(&server_dh_ok.server_nonce, &server_nonce)?;

    if server_dh_ok.encrypted_answer.len() % 16 != 0 {
        return Err(Error::EncryptedResponseNotPadded {
            len: server_dh_ok.encrypted_answer.len(),
        });
    }

    let (key_arr, iv_arr) = generate_key_data_from_nonce(&server_nonce, &new_nonce);
    aes::ige_decrypt(&mut server_dh_ok.encrypted_answer, &key_arr, &iv_arr);
    let plain = server_dh_ok.encrypted_answer;

    let got_hash: [u8; 20] = plain[..20].try_into().unwrap();
    let mut cursor = Cursor::from_slice(&plain[20..]);

    let inner = match ferogram_tl_types::enums::ServerDhInnerData::deserialize(&mut cursor) {
        Ok(ferogram_tl_types::enums::ServerDhInnerData::ServerDhInnerData(x)) => x,
        Err(e) => return Err(Error::InvalidDhInnerData { error: e }),
    };

    let expected_hash: [u8; 20] = {
        let mut sha = Sha1::new();
        sha.update(&plain[20..20 + cursor.pos()]);
        sha.finalize().into()
    };
    if got_hash != expected_hash {
        return Err(Error::InvalidAnswerHash {
            got: got_hash,
            expected: expected_hash,
        });
    }

    check_nonce(&inner.nonce, &nonce)?;
    check_server_nonce(&inner.server_nonce, &server_nonce)?;

    check_p_and_g(&inner.dh_prime, inner.g as u32)
        .map_err(|source| Error::InvalidDhPrime { source })?;

    // Validate g_a range.
    let dh_prime_bn = BigUint::from_bytes_be(&inner.dh_prime);
    let one = BigUint::from(1u32);
    let g_a_bn = BigUint::from_bytes_be(&inner.g_a);
    let safety = one.clone() << (2048 - 64);
    check_g_in_range(&g_a_bn, &safety, &(&dh_prime_bn - &safety))?;

    let dh = DhParamsForRetry {
        dh_prime: inner.dh_prime,
        g: inner.g as u32,
        g_a: inner.g_a,
        server_time: inner.server_time,
        aes_key: key_arr,
        aes_iv: iv_arr,
    };

    generate_client_dh_params(&dh, nonce, server_nonce, new_nonce, retry_id, random, now)
}

// finish: create_key

/// Finalise the handshake.
///
/// Returns [`FinishResult::Done`] on success or [`FinishResult::Retry`] when
/// the server sends `dh_gen_retry` (up to 5 attempts are typical). On retry,
/// call [`retry_step3`] with the returned fields, send the new request, receive
/// the answer, then call `finish` again.
pub fn finish(
    data: Step3,
    response: ferogram_tl_types::enums::SetClientDhParamsAnswer,
) -> Result<FinishResult, Error> {
    let Step3 {
        nonce,
        server_nonce,
        new_nonce,
        time_offset,
        auth_key: auth_key_bytes,
        dh_params,
    } = data;

    struct DhData {
        nonce: [u8; 16],
        server_nonce: [u8; 16],
        hash: [u8; 16],
        num: u8,
    }

    let dh = match response {
        // Variant names come from the constructor names: dh_gen_ok -> DhGenOk, etc.
        ferogram_tl_types::enums::SetClientDhParamsAnswer::DhGenOk(x) => DhData {
            nonce: x.nonce,
            server_nonce: x.server_nonce,
            hash: x.new_nonce_hash1,
            num: 1,
        },
        ferogram_tl_types::enums::SetClientDhParamsAnswer::DhGenRetry(x) => DhData {
            nonce: x.nonce,
            server_nonce: x.server_nonce,
            hash: x.new_nonce_hash2,
            num: 2,
        },
        ferogram_tl_types::enums::SetClientDhParamsAnswer::DhGenFail(x) => DhData {
            nonce: x.nonce,
            server_nonce: x.server_nonce,
            hash: x.new_nonce_hash3,
            num: 3,
        },
    };

    check_nonce(&dh.nonce, &nonce)?;
    check_server_nonce(&dh.server_nonce, &server_nonce)?;

    let auth_key = AuthKey::from_bytes(auth_key_bytes);
    let expected_hash = auth_key.calc_new_nonce_hash(&new_nonce, dh.num);
    check_new_nonce_hash(&dh.hash, &expected_hash)?;

    let first_salt = {
        let mut buf = [0u8; 8];
        for ((dst, a), b) in buf.iter_mut().zip(&new_nonce[..8]).zip(&server_nonce[..8]) {
            *dst = a ^ b;
        }
        i64::from_le_bytes(buf)
    };

    match dh.num {
        1 => Ok(FinishResult::Done(Finished {
            auth_key: auth_key.to_bytes(),
            time_offset,
            first_salt,
        })),
        2 => {
            // dh_gen_retry: compute auth_key_aux_hash = SHA1(auth_key)[0..8] as i64 LE.
            let aux_hash: [u8; 20] = {
                let mut sha = Sha1::new();
                sha.update(auth_key.to_bytes());
                sha.finalize().into()
            };
            let retry_id = i64::from_le_bytes(aux_hash[..8].try_into().unwrap());
            Ok(FinishResult::Retry {
                retry_id,
                dh_params,
                nonce,
                server_nonce,
                new_nonce,
            })
        }
        _ => Err(Error::DhGenFail),
    }
}

// Helpers

fn check_nonce(got: &[u8; 16], expected: &[u8; 16]) -> Result<(), Error> {
    if got == expected {
        Ok(())
    } else {
        Err(Error::InvalidNonce {
            got: *got,
            expected: *expected,
        })
    }
}
fn check_server_nonce(got: &[u8; 16], expected: &[u8; 16]) -> Result<(), Error> {
    if got == expected {
        Ok(())
    } else {
        Err(Error::InvalidServerNonce {
            got: *got,
            expected: *expected,
        })
    }
}
fn check_new_nonce_hash(got: &[u8; 16], expected: &[u8; 16]) -> Result<(), Error> {
    if got == expected {
        Ok(())
    } else {
        Err(Error::InvalidNewNonceHash {
            got: *got,
            expected: *expected,
        })
    }
}
fn check_g_in_range(val: &BigUint, lo: &BigUint, hi: &BigUint) -> Result<(), Error> {
    if lo < val && val < hi {
        Ok(())
    } else {
        Err(Error::GParameterOutOfRange {
            value: val.clone(),
            low: lo.clone(),
            high: hi.clone(),
        })
    }
}

/// RSA key by server fingerprint. Includes both production and test DC keys.
#[allow(clippy::unreadable_literal)]
pub fn key_for_fingerprint(fp: i64) -> Option<rsa::Key> {
    Some(match fp {
        // Production DC key (fingerprint -3414540481677951611)
        -3414540481677951611 => rsa::Key::new(
            "29379598170669337022986177149456128565388431120058863768162556424047512191330847455146576344487764408661701890505066208632169112269581063774293102577308490531282748465986139880977280302242772832972539403531316010870401287642763009136156734339538042419388722777357134487746169093539093850251243897188928735903389451772730245253062963384108812842079887538976360465290946139638691491496062099570836476454855996319192747663615955633778034897140982517446405334423701359108810182097749467210509584293428076654573384828809574217079944388301239431309115013843331317877374435868468779972014486325557807783825502498215169806323",
            "65537",
        )?,
        // Test DC key (fingerprint -5595554452916591101)
        -5595554452916591101 => rsa::Key::new(
            "25342889448840415564971689590713473206898847759084779052582026594546022463853940585885215951168491965708222649399180603818074200620463776135424884632162512403163793083921641631564740959529419359595852941166848940585952337613333022396096584117954892216031229237302943701877588456738335398602461675225081791820393153757504952636234951323237820036543581047826906120927972487366805292115792231423684261262330394324750785450942589751755390156647751460719351439969059949569615302809050721500330239005077889855323917509948255722081644689442127297605422579707142646660768825302832201908302295573257427896031830742328565032949",
            "65537",
        )?,
        _ => return None,
    })
}
