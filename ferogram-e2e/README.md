# ferogram-e2e

Telegram secret chats (end-to-end encryption) for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-e2e?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-e2e)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--e2e-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-e2e)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

Implements Telegram's secret chat protocol (MTProto 2.0): key exchange, message and file encryption, re-keying, and self-destruct timers. `ferogram` re-exports this as `ferogram::e2e` behind the `e2e` feature, so most people never depend on it directly. For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

> **Beta.** This crate is `0.1.0-b1`. The encryption layer has unit tests and the full flow (request, accept, send, receive) has been tried in live secret chats, but it is still young. Expect API changes before `0.1.0`.

## What it does

- Diffie-Hellman key exchange with Telegram's validity checks (`dh`)
- MTProto 2.0 message encryption and decryption, key fingerprints (`crypto`)
- The inner secret chat TL layer: text, photos, documents, locations, contacts, service actions (`tl`)
- Per-file AES-256-IGE encryption plus chunked upload and download of encrypted files (`upload`)
- Chat state: sequence numbers, TTLs, re-keying, gap detection and resend requests (`state`)
- `E2eClient` to request, accept and use secret chats (with ferogram, `request_secret_chat` takes a `@username`, phone or user id and looks up the access hash itself), and `E2eEvent` for what comes back from the update stream
- `E2eTransport` trait, so any client can drive it (the crate does not depend on `ferogram`)

## Usage

With `ferogram`, enable the `e2e` feature:

```toml
ferogram = { version = "0.6", features = ["e2e"] }
```

```rust
use ferogram::e2e::{E2eClient, E2eClientExt, E2eEvent};

// Keeps chats in `mybot.e2e` next to the session, so they survive restarts.
let e2e = E2eClient::for_session(client.clone(), "mybot.session")?;
let chat_id = e2e.request_secret_chat("@username").await?;

let mut stream = client.stream_updates();
while let Some(update) = stream.next().await {
    match e2e.handle_client_update(&update).await? {
        Some(E2eEvent::ChatReady { chat_id }) => {
            e2e.send_text(chat_id, "hello", 0).await?;
        }
        Some(E2eEvent::Message { chat_id, message, .. }) => {
            println!("{chat_id}: {message:?}");
        }
        _ => {}
    }
}
```

A runnable version is in [`ferogram/examples/secret_chat.rs`](https://github.com/ankit-chaubey/ferogram/blob/main/ferogram/examples/secret_chat.rs).

## Storage

`E2eClient::for_session` stores chats in a small binary file, `<session_name>.e2e` (`mybot.session` becomes `mybot.e2e`). Writes are atomic, the file is checksummed, and on Unix it is created with mode `0600`. `E2eClient::new` keeps everything in memory, and `E2eClient::with_persistence(client, path)` takes an explicit path.

The file holds each chat's `auth_key` unencrypted. Keep it somewhere only your process can read.

## Other clients

Implement `E2eTransport` (`invoke` and `invoke_on_dc`) for your client type and feed raw `tl::enums::Update`s into `E2eClient::handle_update`.

## Known limitations

- Secret chats work between users only (not groups or channels), as in Telegram itself.

## Stack position

```
ferogram
└ ferogram-e2e  <-- here
  ├ ferogram-crypto
  └ ferogram-tl-types
```

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
