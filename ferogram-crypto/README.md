# ferogram-crypto

Cryptographic primitives for the Telegram MTProto 2.0 protocol.

[![Crates.io](https://img.shields.io/crates/v/ferogram-crypto?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-crypto)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--crypto-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-crypto)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

AES-IGE, RSA, SHA-1/256, Diffie-Hellman, PQ factorization, auth key derivation, and transport obfuscation. All written specifically for Telegram's protocol.

This is a low-level crate. If you're building a bot or a client, you want [`ferogram`](https://crates.io/crates/ferogram) instead; it uses this internally.

If you need general-purpose crypto in Rust, [RustCrypto](https://github.com/RustCrypto) is the right place.

---

## Modules

### AES-IGE

MTProto uses AES-IGE mode, which isn't in standard crypto libraries. Used by `ferogram-mtproto` to encrypt and decrypt every MTProto message.

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

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
