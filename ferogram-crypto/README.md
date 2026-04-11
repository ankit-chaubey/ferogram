# ferogram-crypto

Cryptographic primitives for the Telegram MTProto 2.0 protocol.

[![Crates.io](https://img.shields.io/crates/v/ferogram-crypto?color=fc8d62)](https://crates.io/crates/ferogram-crypto)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--crypto-5865F2)](https://docs.rs/ferogram-crypto)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)

Implements AES-IGE, RSA, SHA-1/256, Diffie-Hellman, PQ factorization, auth key derivation, and transport obfuscation. All algorithms are written from scratch to match Telegram's specification.

---

## Installation

```toml
[dependencies]
ferogram-crypto = "0.1.1"
```

---

## Modules

### AES-IGE

MTProto uses AES-IGE mode, not available in standard crypto libraries. Used by `ferogram-mtproto` to encrypt and decrypt every MTProto message.

```rust
use ferogram_crypto::aes::{ige_encrypt, ige_decrypt};

// key: 32 bytes, iv: 32 bytes
let ciphertext = ige_encrypt(&plaintext, &key, &iv);
let recovered  = ige_decrypt(&ciphertext, &key, &iv);
```

### RSA

Encrypts `p_q_inner_data` with Telegram's server public key during the DH handshake. Uses `num-bigint` for modular exponentiation.

```rust
use ferogram_crypto::rsa::encrypt;
let encrypted = encrypt(&data, &public_key_modulus, &public_key_exponent);
```

### SHA

```rust
use ferogram_crypto::sha::{sha1, sha256};

let hash1 = sha1(&data);   // [u8; 20]
let hash2 = sha256(&data); // [u8; 32]
```

SHA-1 is used in auth key derivation and older `msg_key` paths. SHA-256 is used in MTProto 2.0 `msg_key` derivation.

### PQ Factorization

The server sends a product `pq` during DH Step 1 that the client must factor. Uses Pollard's rho algorithm, O(n^1/4) expected time.

```rust
use ferogram_crypto::factorize::factorize;

let (p, q) = factorize(0x17ED48941A08F981_u64);
// p * q == pq, p < q, both prime
```

### Auth Key Derivation

After DH exchange, the raw shared secret is expanded into the 2048-bit auth key using Telegram's SHA-1-based KDF. Runs inside `ferogram-mtproto`'s `authentication::finish()`.

### Diffie-Hellman

`g^a mod p` and `g^(ab) mod p` computed via `num-bigint`. Parameters received from the server are validated before use.

### Transport Obfuscation

`ObfuscatedCodec` XOR-encrypts all bytes over the TCP connection to resist protocol fingerprinting.

```rust
use ferogram_crypto::obfuscated::ObfuscatedCodec;

let (codec, init_bytes) = ObfuscatedCodec::new()?;
// Send init_bytes to server first, then use codec for all subsequent I/O
```

---

## Stack position

```
ferogram
└ ferogram-mtproto
  ├ ferogram-tl-types
  └ ferogram-crypto  <-- here
```

---

## Note

This crate is purpose-built for MTProto. For general-purpose Rust crypto, use [RustCrypto](https://github.com/RustCrypto).

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
