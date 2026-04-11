# Error Types

---

## `InvocationError`

Every `Client` method returns `Result<T, InvocationError>`.

```rust
use ferogram::{InvocationError, RpcError};

match client.send_message("@peer", "Hello").await {
    Ok(()) => {}

    // Telegram returned an RPC error
    Err(InvocationError::Rpc(e)) => {
        eprintln!("Telegram error {}: {}", e.code, e.message);

        // Pattern-match the error name
        if e.is("FLOOD_WAIT") {
            let secs = e.flood_wait_seconds().unwrap_or(0);
            eprintln!("Rate limited, wait {secs}s");
        }
        if e.is("USER_PRIVACY_RESTRICTED") {
            eprintln!("Can't message this user");
        }
    }

    // Network / I/O failure
    Err(InvocationError::Io(e)) => eprintln!("I/O: {e}"),

    Err(e) => eprintln!("Other: {e}"),
}
```

### `InvocationError` variants

| Variant | Description |
|---|---|
| `InvocationError::Rpc(RpcError)` | Telegram returned a TL error |
| `InvocationError::Io(io::Error)` | Network or socket error |
| `InvocationError::Deserialize(e)` | Failed to decode server response |

### `InvocationError` methods

| Method | Return | Description |
|---|---|---|
| `.is("PATTERN")` | `bool` | `true` if this is an `Rpc` error whose message contains `PATTERN` |
| `.flood_wait_seconds()` | `Option<u64>` | Seconds to wait for `FLOOD_WAIT_X` errors |

---

## `RpcError`

```rust
let e: RpcError = /* ... */;

e.code               // i32: HTTP-like error code (400, 401, 403, 420, etc.)
e.message            // String: e.g. "FLOOD_WAIT_30"
e.is("FLOOD_WAIT")   // bool: prefix/substring match
e.flood_wait_seconds() // Option<u64>: parses the number from FLOOD_WAIT_N
```

### Common error codes

| Code | Meaning |
|---|---|
| `400` | Bad request: wrong parameters |
| `401` | Unauthorized: not logged in |
| `403` | Forbidden: no permission |
| `404` | Not found |
| `420` | FLOOD_WAIT: too many requests |
| `500` | Internal server error |

### Common error messages

| Message | Meaning |
|---|---|
| `FLOOD_WAIT_N` | Wait N seconds before retrying |
| `USER_PRIVACY_RESTRICTED` | User's privacy settings block this |
| `CHAT_WRITE_FORBIDDEN` | No permission to write in this chat |
| `MESSAGE_NOT_MODIFIED` | Edit content is the same as current |
| `PEER_ID_INVALID` | The peer ID is wrong or missing access hash |
| `USER_ID_INVALID` | Invalid user ID or no access hash |
| `CHANNEL_INVALID` | Invalid channel or missing access hash |
| `SESSION_REVOKED` | Session was logged out remotely |
| `AUTH_KEY_UNREGISTERED` | Auth key no longer valid |

---

## `SignInError`

Returned by `client.sign_in()`:

```rust
use ferogram::SignInError;

match client.sign_in(&token, &code).await {
    Ok(name) => println!("Welcome, {name}!"),

    Err(SignInError::PasswordRequired(password_token)) => {
        // 2FA is enabled: provide the password
        println!("2FA hint: {:?}", password_token.hint());
        client.check_password(*password_token, "my_password").await?;
    }

    Err(SignInError::InvalidCode) => eprintln!("Wrong code"),

    Err(SignInError::Other(e)) => eprintln!("Error: {e}"),
}
```

### `PasswordToken` methods

```rust
password_token.hint()   // Option<&str>: 2FA password hint
```

---

## `BuilderError`

Returned by `ClientBuilder::connect()` or `ClientBuilder::build()`:

```rust
use ferogram::builder::BuilderError;

match Client::builder().api_id(0).api_hash("").connect().await {
    Err(BuilderError::MissingApiId)   => eprintln!("Set .api_id()"),
    Err(BuilderError::MissingApiHash) => eprintln!("Set .api_hash()"),
    Err(BuilderError::Connect(e))     => eprintln!("Connection failed: {e}"),
    Ok(_) => {}
}
```

---

## FLOOD_WAIT auto-retry

`FLOOD_WAIT` errors are automatically retried by the default `AutoSleep` policy: you don't need to handle them unless you want custom behaviour.

```rust
use ferogram::retry::{AutoSleep, NoRetries};
use std::sync::Arc;

// Default: retries FLOOD_WAIT automatically
Client::builder().retry_policy(Arc::new(AutoSleep::default()))

// Disable all retries
Client::builder().retry_policy(Arc::new(NoRetries))
```
