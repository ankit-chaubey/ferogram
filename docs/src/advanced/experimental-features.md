# Experimental Features

`ExperimentalFeatures` is a struct that holds opt-in flags for behaviours that
deviate from the strict Telegram MTProto spec. Every flag defaults to `false`.
Enable only what you need after reading the warnings below.

Pass the struct to `.experimental_features()` on the builder:

```rust
use ferogram::{Client, ExperimentalFeatures};

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .experimental_features(ExperimentalFeatures {
        allow_zero_hash: true,
        ..Default::default()
    })
    .connect()
    .await?;
```

---

## Flags

### `allow_zero_hash`

**Default:** `false`  
**Safe for:** bot accounts only

When no `access_hash` is cached for a user or channel, fall back to
`access_hash = 0` instead of returning `InvocationError::PeerNotCached`.

The Telegram spec explicitly permits `hash = 0` for bot accounts when only a
min-hash is available. Bot tokens receive this entitlement from the server
automatically. On user accounts, sending `hash = 0` produces
`USER_ID_INVALID` or `CHANNEL_INVALID`.

```rust
// Bot receiving a message from a user it has never seen before.
// Without this flag, calling client.send_message(user_id, ...) would
// fail with PeerNotCached because no access_hash is in the cache yet.
// With allow_zero_hash the request goes out with hash=0 and succeeds.
ExperimentalFeatures {
    allow_zero_hash: true,
    ..Default::default()
}
```

**When to use:** Enable this if your bot handles `updateShortMessage`,
`updateShortChatMessage`, or other compact update types that carry only a
`user_id` / `chat_id` without an `access_hash`. These updates arrive before
the bot has a chance to cache the peer's full info.

**Do not enable** on user (non-bot) accounts. The server will reject the request.

---

### `allow_missing_channel_hash`

**Default:** `false`  
**Safe for:** debugging and testing only

When resolving a min-user via `InputPeerUserFromMessage`, if the containing
channel's `access_hash` is not in the cache, proceed with
`channel access_hash = 0` rather than returning `InvocationError::PeerNotCached`.

In practice this is almost always wrong. The inner
`InputPeerChannel { access_hash: 0 }` makes the entire
`InputPeerUserFromMessage` invalid and Telegram will reject it with
`CHANNEL_INVALID`. The flag exists solely for debugging peer resolution
without triggering the cache-miss guard.

```rust
ExperimentalFeatures {
    allow_missing_channel_hash: true,  // debugging only
    ..Default::default()
}
```

**Do not enable** in production.

---

### `auto_resolve_peers`

**Default:** `false`  
**Status:** reserved, not yet implemented

When set, a cache miss would automatically call `users.getUsers` /
`channels.getChannels` to fetch a fresh `access_hash` before constructing the
`InputPeer`. Currently this flag has no effect. It is reserved for a future
release that will add automatic peer hydration.

---

## Combining flags

All flags are independent. Use `..Default::default()` to leave the rest at
`false`:

```rust
ExperimentalFeatures {
    allow_zero_hash: true,
    allow_missing_channel_hash: false,  // explicit, or just use ..Default::default()
    auto_resolve_peers: false,
    ..Default::default()  // forward-compatible: new flags stay false
}
```

Always use `..Default::default()` so that new flags added in future versions
default to `false` without requiring changes to your code.

---

## Relationship to `PeerNotCached`

Without any experimental flags, a cache miss on an `access_hash` returns:

```
InvocationError::PeerNotCached { peer_id: 123456789 }
```

The correct fix in most cases is to ensure the peer appears in an update or
API response before you try to address it. For bots, enabling `allow_zero_hash`
is the idiomatic workaround for compact update types.

```rust
// Typical bot pattern: handle updateShortMessage
// user_id is known, but no access_hash yet
match client.send_message(user_id, "hello").await {
    Err(InvocationError::PeerNotCached { .. }) => {
        // cache not warm yet, would not happen with allow_zero_hash
    }
    Ok(_) => {}
    Err(e) => return Err(e),
}
```
