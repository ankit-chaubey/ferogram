# Secret Chats

Secret chats are end-to-end encrypted one-to-one chats (MTProto 2.0). They are available behind the `e2e` feature, which pulls in the [`ferogram-e2e`](https://crates.io/crates/ferogram-e2e) crate and exposes it as `ferogram::e2e`.

```toml
ferogram = { version = "0.6", features = ["e2e"] }
```

## Start a chat

```rust
use ferogram::e2e::{E2eClient, E2eClientExt, E2eEvent};

// Chats are kept in `mybot.e2e` next to the session, so they survive restarts.
let e2e = E2eClient::for_session(client.clone(), "mybot.session")?;

// "@username", a phone number or a user id. The access hash is looked up for you.
let chat_id = e2e.request_secret_chat("@username").await?;
```

The chat is usable once the other person accepts it and an `E2eEvent::ChatReady` arrives.

## Handle updates

Secret chat traffic arrives through the normal update stream. Pass every update to `handle_client_update`:

```rust
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

Incoming chat requests are accepted automatically and reported as `E2eEvent::ChatRequested`, followed by `ChatReady`.

## Events

| Event | Meaning |
|---|---|
| `ChatRequested` | Someone asked for a secret chat with you (already accepted) |
| `ChatReady` | The key exchange finished, the chat can be used |
| `ChatDiscarded` | The other side ended the chat |
| `Message` | A decrypted message, with the encrypted file reference if it has media |
| `MessageExpired` | A self-destruct timer ran out |

## What you can do

| Method | Purpose |
|---|---|
| `send_text(chat_id, text, ttl)` | Send text, with an optional self-destruct timer in seconds (`0` for none) |
| `send_photo(...)`, `send_file(...)` | Encrypt and upload media |
| `download_decrypted_file(...)` | Download and decrypt media from a `Message` event |
| `set_encrypted_typing(chat_id, typing)` | Typing indicator |
| `emoji_fingerprint(chat_id)` | Four emojis both people can compare to check the key |
| `initiate_rekey(chat_id)` | Start a key re-exchange |
| `discard(chat_id, delete_history)` | End the chat |
| `subscribe()` | Receive events that are not tied to an update, such as `MessageExpired` |

## Storage

`E2eClient::for_session` stores chats in a small binary file, `<session_name>.e2e` (`mybot.session` becomes `mybot.e2e`). Writes are atomic, the file is checksummed, and on Unix it is created with mode `0600`. `E2eClient::new` keeps chats in memory only, and `E2eClient::with_persistence(client, path)` takes an explicit path.

The file holds each chat's key unencrypted, so keep it as private as your session file.

## Limits

- Secret chats are one-to-one between users. Bots, groups and channels can't use them.
- As in Telegram itself, a secret chat belongs to one device.
- Running a client other than ferogram? Implement `E2eTransport` for it and call `E2eClient::handle_update` with raw updates.

See the [`secret_chat` example](https://github.com/ankit-chaubey/ferogram/blob/main/ferogram/examples/secret_chat.rs) for a complete program.
