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

#[allow(deprecated)]
use aes::cipher::{KeyIvInit, StreamCipher, generic_array::GenericArray};

/// AES-256-CTR stream cipher pair for MTProto obfuscated transport.
pub struct ObfuscatedCipher {
    #[allow(deprecated)]
    rx: ctr::Ctr128BE<aes::Aes256>,
    #[allow(deprecated)]
    tx: ctr::Ctr128BE<aes::Aes256>,
}

impl ObfuscatedCipher {
    /// Build cipher state from the 64-byte random init buffer.
    #[allow(deprecated)]
    pub fn new(init: &[u8; 64]) -> Self {
        let rev: Vec<u8> = init.iter().copied().rev().collect();
        Self {
            rx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(&rev[8..40]),
                GenericArray::from_slice(&rev[40..56]),
            ),
            tx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(&init[8..40]),
                GenericArray::from_slice(&init[40..56]),
            ),
        }
    }

    /// Build cipher from explicit key/IV pairs (used when MTProxy secret
    /// mixing has already been applied externally via SHA-256).
    #[allow(deprecated)]
    pub fn from_keys(
        tx_key: &[u8; 32],
        tx_iv: &[u8; 16],
        rx_key: &[u8; 32],
        rx_iv: &[u8; 16],
    ) -> Self {
        Self {
            tx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(tx_key),
                GenericArray::from_slice(tx_iv),
            ),
            rx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(rx_key),
                GenericArray::from_slice(rx_iv),
            ),
        }
    }

    /// Encrypt outgoing bytes in-place (TX direction).
    pub fn encrypt(&mut self, buf: &mut [u8]) {
        self.tx.apply_keystream(buf);
    }

    /// Decrypt incoming bytes in-place (RX direction).
    pub fn decrypt(&mut self, buf: &mut [u8]) {
        self.rx.apply_keystream(buf);
    }
}

/// Generate the 64-byte obfuscated init buffer and build the cipher for it.
///
/// `framing_byte`: 0xef = Abridged, 0xdd = PaddedIntermediate.
/// `proxy_secret`: if present, SHA-256 mixes the key with the secret (MTProxy).
///
/// Returns `(nonce, cipher)`. The caller writes `nonce` to the stream; the
/// cipher is used for all subsequent I/O on that connection.
pub fn build_obfuscated_init(
    framing_byte: u8,
    dc_id: i16,
    proxy_secret: Option<&[u8]>,
) -> ([u8; 64], ObfuscatedCipher) {
    use sha2::Digest;

    let mut nonce = [0u8; 64];
    loop {
        crate::fill_random(&mut nonce);
        let first = u32::from_le_bytes(nonce[0..4].try_into().expect("4-byte slice"));
        let second = u32::from_le_bytes(nonce[4..8].try_into().expect("4-byte slice"));
        let bad = nonce[0] == 0xEF
            || first == 0x44414548 // HEAD
            || first == 0x54534F50 // POST
            || first == 0x20544547 // GET
            || first == 0x4954504f // OPTIONS
            || first == 0xEEEEEEEE
            || first == 0xDDDDDDDD
            || first == 0x02010316
            || second == 0x00000000;
        if !bad {
            break;
        }
    }

    let tx_raw: [u8; 32] = nonce[8..40].try_into().expect("32-byte slice");
    let tx_iv: [u8; 16] = nonce[40..56].try_into().expect("16-byte slice");
    let mut rev48 = nonce[8..56].to_vec();
    rev48.reverse();
    let rx_raw: [u8; 32] = rev48[0..32].try_into().expect("32-byte slice");
    let rx_iv: [u8; 16] = rev48[32..48].try_into().expect("16-byte slice");

    let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = proxy_secret {
        let mut h = sha2::Sha256::new();
        h.update(tx_raw);
        h.update(s);
        let tx: [u8; 32] = h.finalize().into();

        let mut h = sha2::Sha256::new();
        h.update(rx_raw);
        h.update(s);
        let rx: [u8; 32] = h.finalize().into();
        (tx, rx)
    } else {
        (tx_raw, rx_raw)
    };

    nonce[56] = framing_byte;
    nonce[57] = framing_byte;
    nonce[58] = framing_byte;
    nonce[59] = framing_byte;
    let dc_bytes = dc_id.to_le_bytes();
    nonce[60] = dc_bytes[0];
    nonce[61] = dc_bytes[1];

    let mut cipher = ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
    let mut skip = [0u8; 56];
    cipher.encrypt(&mut skip);
    cipher.encrypt(&mut nonce[56..64]);

    (nonce, cipher)
}

/// Compute the FakeTLS ClientHello random-field value.
///
/// `secret` is the 16-byte MTProxy key. `record` must be the fully assembled
/// ClientHello TLS record with the 32-byte random field zeroed.
///
/// Returns `HMAC-SHA256(secret, record)` with its last 4 bytes XORed with the
/// current unix timestamp (little-endian) -- the anti-replay scheme real
/// MTProxy FakeTLS servers check for. The caller writes this value into the
/// record's random field, then sends the record as-is.
pub fn fake_tls_client_digest(secret: &[u8; 16], record: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC key error: secret must be non-empty");
    mac.update(record);
    let mut digest: [u8; 32] = mac.finalize().into_bytes().into();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let last4 = u32::from_le_bytes(digest[28..32].try_into().expect("4-byte slice")) ^ now;
    digest[28..32].copy_from_slice(&last4.to_le_bytes());
    digest
}

/// Verify a FakeTLS ServerHello digest.
///
/// `packet_with_digest_zeroed` must be the concatenated ServerHello +
/// ChangeCipherSpec + first Application-Data ("cert") record bytes, exactly
/// as read off the wire, with the server's own 32-byte digest field zeroed
/// in place. `client_digest` is the raw value this client sent on the wire
/// in its own ClientHello. `expected_digest` is what the server actually
/// sent (extracted before zeroing). Returns `true` if the server proves
/// knowledge of the secret.
pub fn fake_tls_verify_server_digest(
    secret: &[u8; 16],
    client_digest: &[u8; 32],
    packet_with_digest_zeroed: &[u8],
    expected_digest: &[u8; 32],
) -> bool {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC key error: secret must be non-empty");
    mac.update(client_digest);
    mac.update(packet_with_digest_zeroed);
    let computed: [u8; 32] = mac.finalize().into_bytes().into();
    computed == *expected_digest
}
