# Quick Start: User Account

A complete working example: connect, log in, send a message to Saved Messages, and listen for incoming messages.

```rust
use ferogram::Client;
use ferogram::update::Update;

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, _shutdown) = Client::quick_connect("my.session", API_ID, API_HASH).await?;

    // Send a message to yourself
    client.send_to_self("Hello from ferogram! 👋").await?;
    println!("Message sent to Saved Messages");

    // Stream incoming updates
    println!("Listening for messages… (Ctrl+C to quit)");
    let mut updates = client.stream_updates();

    while let Some(update) = updates.next().await {
        match update {
            Update::NewMessage(msg) if !msg.outgoing() => {
                let text   = msg.text().unwrap_or("(no text)");
                let sender = msg.sender_id()
                    .map(|p| format!("{p:?}"))
                    .unwrap_or_else(|| "unknown".into());

                println!("📨 [{sender}] {text}");
            }
            Update::MessageEdited(msg) => {
                println!("✏️  Edited: {}", msg.text().unwrap_or(""));
            }
            _ => {}
        }
    }

    Ok(())
}
```

---

## Run it

```bash
cargo run
```

Fill in `API_ID` and `API_HASH` at the top of the file before running. On first run you'll be prompted for your phone number and the code Telegram sends. On subsequent runs the session is reloaded from `my.session` and login is skipped automatically.

---

## What each step does

| Step | Method | Description |
|---|---|---|
| Connect + auth | `Client::quick_connect` | Opens TCP, DH handshake, loads session, prompts for phone/code/2FA or bot token if not yet authorized, saves session |
| Stream | `stream_updates` | Returns an `UpdateStream` async iterator |

For the full auth flow broken down step by step, see [User Login](./authentication/user-login.md).

---

## Next steps

- [User Login: full guide](./authentication/user-login.md)
- [Two-Factor Auth (2FA)](./authentication/2fa.md)
- [Session Backends](./authentication/session-backends.md): string sessions, SQLite, Turso
- [Update Types](./updates/update-types.md)
