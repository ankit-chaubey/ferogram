# ferogram-mtproto

MTProto 2.0 session management, DH key exchange, and message framing for Rust.

[![Crates.io](https://img.shields.io/crates/v/ferogram-mtproto?color=fc8d62)](https://crates.io/crates/ferogram-mtproto)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--mtproto-5865F2)](https://docs.rs/ferogram-mtproto)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6)](https://core.telegram.org/mtproto)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)

---

## Installation

```toml
[dependencies]
ferogram-mtproto  = "0.4.8"
ferogram-tl-types = { version = "0.4.8", features = ["tl-mtproto"] }
```

---

## Overview

`ferogram-mtproto` implements the full MTProto 2.0 session layer. It handles:

- 3-step DH key exchange
- Encrypted sessions (AES-IGE pack/unpack)
- Message framing: salt, session_id, message_id, sequence numbers
- `msg_container` and `gzip_packed` support
- Salt management and auto-correction
- Error recovery: `bad_msg_notification`, `bad_server_salt`, `msg_resend_req`
- Acknowledgements via `MsgsAck`

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

### Session (plaintext)

Used only before an auth key exists, for the initial DH handshake messages.

```rust
use ferogram_mtproto::Session;

let mut plain = Session::new();
let framed = plain.pack_plain(&my_handshake_request)?;
```

---

## Message Framing

Every outgoing MTProto message:

```
server_salt     8 bytes
session_id      8 bytes
message_id      8 bytes  (Unix time * 2^32, monotonically increasing)
seq_no          4 bytes
message_length  4 bytes
payload         N bytes
padding         12-1024 random bytes (total % 16 == 0)
```

Prefixed by a 32-byte `msg_key` after encryption.

`msg_key` is SHA-256 of `(auth_key[88..120] || plaintext)` for client-to-server, and `(auth_key[96..128] || plaintext)` for server-to-client.

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
