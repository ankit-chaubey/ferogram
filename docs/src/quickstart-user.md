# Quick Start: User Account

A complete working example: connect, log in, send a message to Saved Messages, and listen for incoming messages.

```rust
use ferogram::{Client, Config, SignInError};
use ferogram::update::Update;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, _shutdown) = Client::builder()
        .api_id(std::env::var("TG_API_ID")?.parse()?)
        .api_hash(std::env::var("TG_API_HASH")?)
        .session("my.session")
        .connect()
        .await?;

    // Login (skipped if session file already has valid auth)
    if !client.is_authorized().await? {
        print!("Phone number (+1234567890): ");
        io::stdout().flush()?;
        let phone = read_line();

        let token = client.request_login_code(&phone).await?;

        print!("Verification code: ");
        io::stdout().flush()?;
        let code = read_line();

        match client.sign_in(&token, &code).await {
            Ok(name) => println!("✅ Signed in as {name}"),
            Err(SignInError::PasswordRequired(pw_token)) => {
                print!("2FA password: ");
                io::stdout().flush()?;
                let pw = read_line();
                client.check_password(pw_token, &pw).await?;
                println!("✅ 2FA verified");
            }
            Err(e) => return Err(e.into()),
        }
        client.save_session().await?;
    }

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

fn read_line() -> String {
    let mut s = String::new();
    io::stdin().read_line(&mut s).unwrap();
    s.trim().to_string()
}
```

---

## Run it

```bash
TG_API_ID=12345 TG_API_HASH=yourHash cargo run
```

On first run you'll be prompted for your phone number and the code Telegram sends. On subsequent runs, the session is reloaded from `my.session` and login is skipped automatically.

---

## What each step does

| Step | Method | Description |
|---|---|---|
| Connect | `Client::builder().connect()` | Opens TCP, performs DH handshake, loads session |
| Check auth | `is_authorized` | Returns `true` if session has a valid logged-in user |
| Request code | `request_login_code` | Sends SMS/app code to the phone |
| Sign in | `sign_in` | Submits the code. Returns `PasswordRequired` if 2FA is on |
| 2FA | `check_password` | Performs SRP exchange: password never sent in plain text |
| Save | `save_session` | Writes auth key + DC info to disk |
| Stream | `stream_updates` | Returns an `UpdateStream` async iterator |

---

## Next steps

- [User Login: full guide](./authentication/user-login.md)
- [Two-Factor Auth (2FA)](./authentication/2fa.md)
- [Session Backends](./authentication/session-backends.md): string sessions, SQLite, Turso
- [Update Types](./updates/update-types.md)
