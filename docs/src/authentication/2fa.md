# Two-Factor Authentication (2FA)

Telegram's 2FA uses **Secure Remote Password (SRP)**: a zero-knowledge proof. Your password is never sent to Telegram's servers; only a cryptographic proof is transmitted.

## How it works in ferogram

```rust
match client.sign_in(&login_token, &code).await {
    Ok(name) => {
        // ✅ No 2FA: login complete
        println!("Welcome, {name}!");
    }
    Err(SignInError::PasswordRequired(password_token)) => {
        // 2FA is enabled: the password_token carries SRP parameters
        client.check_password(password_token, "my_2fa_password").await?;
        println!("✅ 2FA verified");
    }
    Err(e) => return Err(e.into()),
}
```

`check_password` performs the full SRP computation internally:

1. Downloads SRP parameters from Telegram (`account.getPassword`)
2. Derives a verifier from your password using PBKDF2-SHA512
3. Computes the SRP proof and sends it (`auth.checkPassword`)

## Getting the password hint

The `PasswordToken` gives you access to the hint the user set when enabling 2FA:

```rust
Err(SignInError::PasswordRequired(token)) => {
    let hint = token.hint().unwrap_or("no hint set");
    println!("Enter your 2FA password (hint: {hint}):");
    let pw = read_line();
    client.check_password(token, &pw).await?;
}
```

## Changing the 2FA password

> **NOTE:** Changing 2FA password requires calling `account.updatePasswordSettings` via raw API. This is an advanced operation: see [Raw API Access](../advanced/raw-api.md).

## Wrong password errors

```rust
use ferogram::{InvocationError, RpcError};

match client.check_password(token, &pw).await {
    Ok(_) => println!("✅ OK"),
    Err(InvocationError::Rpc(RpcError { message, .. }))
        if message.contains("PASSWORD_HASH_INVALID") =>
    {
        println!("❌ Wrong password. Try again.");
    }
    Err(e) => return Err(e.into()),
}
```

## Security notes

- `ferogram-crypto` implements the SRP math from scratch: no external SRP library
- The password derivation uses PBKDF2-SHA512 with 100,000+ iterations
- The SRP exchange is authenticated: a MITM cannot substitute their own verifier
