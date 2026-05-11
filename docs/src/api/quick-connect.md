# quick_connect

`Client::quick_connect` connects and authenticates in a single call, handling
the full auth flow interactively from stdin. If the session is already
authorized the prompt is skipped entirely.

For advanced options (proxy, PFS, custom transport, catch-up, etc.) use
[`ClientBuilder`](./client-builder.md) directly.

---

## Signature

```rust,no_run
pub async fn quick_connect(
    session: impl AsRef<Path>,
    api_id: i32,
    api_hash: &str,
) -> Result<(Client, ShutdownToken), QuickConnectError>
```

---

## Usage

```rust,no_run
use ferogram::Client;

const API_ID: i32 = 12345;
const API_HASH: &str = "your_api_hash";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, _shutdown) = Client::quick_connect("my.session", API_ID, API_HASH).await?;
    // client is ready to use
    Ok(())
}
```

When run, the prompt sequence looks like this:

- Already signed in: no prompt, returns immediately.
- User account: asks for phone number, then login code, then 2FA password if required.
- Bot: paste a bot token (`123456789:AABBcc...`) instead of a phone number.

The bot token is detected automatically by its `<digits>:<string>` format, so
the same prompt works for both users and bots.

---

## Error Handling

`QuickConnectError` covers every failure mode:

| Variant | When it fires |
|---|---|
| `Builder(BuilderError)` | `ClientBuilder::connect` failed (bad credentials or network error) |
| `Auth(InvocationError)` | An MTProto RPC call during auth failed |
| `InvalidCode` | Wrong login code entered |
| `SignUpRequired` | Phone number not registered on Telegram |
| `Io(std::io::Error)` | Failed to read from stdin |

```rust,no_run
use ferogram::client::QuickConnectError;

match Client::quick_connect("my.session", API_ID, API_HASH).await {
    Ok((client, _)) => { /* use client */ }
    Err(QuickConnectError::InvalidCode) => eprintln!("Wrong code, try again"),
    Err(QuickConnectError::SignUpRequired) => eprintln!("Phone not registered"),
    Err(QuickConnectError::Auth(e)) => eprintln!("Auth error: {e}"),
    Err(e) => eprintln!("Connect failed: {e}"),
}
```

---

## When to use ClientBuilder instead

`quick_connect` is intentionally minimal. Reach for
[`ClientBuilder`](./client-builder.md) when you need any of the following:

- SOCKS5 or MTProxy
- Perfect Forward Secrecy (`.pfs(true)`)
- Transport probing or resilient connect
- Custom session backend (e.g. `LibSqlBackend`)
- Catch-up on missed updates (`.catch_up(true)`)
- Custom retry or reconnect policy
- Non-interactive auth (reading credentials from env vars or a config file)
