# DC Migration

Telegram's infrastructure is split across multiple **Data Centers (DCs)**. When you connect to the wrong DC for your account, Telegram responds with a `PHONE_MIGRATE_X` or `USER_MIGRATE_X` error telling you which DC to use instead.

`ferogram` handles DC migration **automatically and transparently**. You don't need to do anything.

## How it works

1. You connect to DC2 (the default)
2. You log in with a phone number registered on DC1
3. Telegram returns `PHONE_MIGRATE_1`
4. `ferogram` reconnects to DC1, re-performs the DH handshake, and retries your request
5. Your code sees a successful response: the migration is invisible

The correct DC is then saved in the session file for future connections.


*Each new DC connection performs a full DH key exchange to establish a fresh auth key for that DC.*

## Overriding the initial DC

By default, `ferogram` starts on DC2. If you know your account is on a different DC, you can set the initial address:

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("my.session")
    .dc_addr("149.154.167.91:443")   // DC4
    .connect()
    .await?;
```

DC addresses:

| DC | Primary IP | Notes |
|---|---|---|
| DC1 | 149.154.175.53 | US East |
| DC2 | 149.154.167.51 | US East (default) |
| DC3 | 149.154.175.100 | US West |
| DC4 | 149.154.167.91 | EU Amsterdam |
| DC5 | 91.108.56.130 | Singapore |

In practice, just leave the default and let auto-migration handle it.

## DC pool (for media)

When downloading media, Telegram may route large files through CDN DCs different from your account's home DC. `ferogram` maintains a connection pool across DCs and handles this automatically via `invoke_on_dc`.
