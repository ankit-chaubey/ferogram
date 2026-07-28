# ferogram-mtproto

MTProto 2.0 session management, DH key exchange, and message framing for Rust.

[![Crates.io](https://img.shields.io/crates/v/ferogram-mtproto?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-mtproto)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--mtproto-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-mtproto)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-227-8B5CF6?style=flat-square)](https://core.telegram.org/mtproto)

The MTProto session layer. Handles everything from raw bytes to decrypted, sequenced messages. `ferogram` sits on top of this; most users don't need to depend on it directly.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

---

## What it handles

- 3-step DH key exchange (`req_pq_multi` → `req_DH_params` → `set_client_DH_params`)
- Encrypted sessions: AES-IGE pack/unpack, msg_key derivation, salt management
- Message framing: salt, session_id, message_id, sequence numbers
- `msg_container` and `gzip_packed` unwrapping
- Error recovery: `bad_msg_notification`, `bad_server_salt`, `msg_resend_req`
- Acknowledgements via `MsgsAck`
- Temporary key binding for PFS (`bind_temp_key` module)

---

## Core Types

### EncryptedSession

Manages the live MTProto session after key exchange.

```rust
use ferogram_mtproto::EncryptedSession;

let session = EncryptedSession::new(auth_key, first_salt, time_offset);

let wire_bytes = session.pack(&my_request)?;
let wire_bytes = session.pack_serializable(&raw_obj)?;
let msg = session.unpack(&mut raw_bytes)?;
```

### 3-Step DH Handshake

```rust
use ferogram_mtproto::authentication as auth;

let (req1, state1) = auth::step1()?;
// send req1, receive res_pq

let (req2, state2) = auth::step2(state1, res_pq)?;
// send req2, receive server_DH_params

let (req3, state3) = auth::step3(state2, dh_params)?;
// send req3, receive dh_answer

let done = auth::finish(state3, dh_answer)?;
// done.auth_key    [u8; 256]
// done.first_salt  i64
// done.time_offset i32
```

### Message

```rust
pub struct Message {
    pub msg_id:  i64,
    pub seq_no:  i32,
    pub salt:    i64,
    pub body:    Vec<u8>,  // raw TL bytes
}
```

### bind_temp_key

Implements PFS temporary key binding.

```rust
use ferogram_mtproto::{serialize_bind_temp_auth_key, encrypt_bind_inner, gen_msg_id};

let wire = serialize_bind_temp_auth_key(
    &perm_auth_key,
    &temp_auth_key,
    nonce,
    expires_at,
    msg_id,
    seq_no,
)?;
```

---

## Stack position

```
ferogram
└ ferogram-mtproto  <-- here
  ├ ferogram-tl-types (tl-mtproto feature)
  └ ferogram-crypto
```

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
