# Raw API Access

Every Telegram API method is available as a typed struct in `ferogram_tl_types::functions`. Use `client.invoke()` to call any of them directly with full compile-time type safety.

## Basic usage

```rust
use ferogram_tl_types::functions;

// Get current update state
let state = client.invoke(
    &functions::updates::GetState {}
).await?;

println!("pts={} qts={} seq={}", state.pts, state.qts, state.seq);
```

## Navigation

All 500+ functions are organized by namespace matching the TL schema:

| TL namespace | Rust path | Examples |
|---|---|---|
| `auth.*` | `functions::auth::` | `SendCode`, `SignIn`, `LogOut` |
| `account.*` | `functions::account::` | `GetPrivacy`, `UpdateProfile` |
| `users.*` | `functions::users::` | `GetFullUser`, `GetUsers` |
| `contacts.*` | `functions::contacts::` | `Search`, `GetContacts`, `AddContact` |
| `messages.*` | `functions::messages::` | `SendMessage`, `GetHistory`, `Search` |
| `updates.*` | `functions::updates::` | `GetState`, `GetDifference` |
| `photos.*` | `functions::photos::` | `UploadProfilePhoto`, `GetUserPhotos` |
| `upload.*` | `functions::upload::` | `SaveFilePart`, `GetFile` |
| `channels.*` | `functions::channels::` | `GetParticipants`, `EditAdmin` |
| `bots.*` | `functions::bots::` | `SetBotCommands`, `GetBotCommands` |
| `payments.*` | `functions::payments::` | `GetStarGiftAuctionState` (L223) |
| `stories.*` | `functions::stories::` | `GetStories`, `CreateAlbum` (L223) |

## Examples

### Get full user info

```rust
use ferogram_tl_types::{functions, enums, types};

let user_full = client.invoke(&functions::users::GetFullUser {
    id: enums::InputUser::InputUser(types::InputUser {
        user_id:     target_user_id,
        access_hash: user_access_hash,
    }),
}).await?;

let tl::enums::users::UserFull::UserFull(uf) = user_full;
if let enums::UserFull::UserFull(info) = uf.full_user {
    println!("About: {:?}", info.about);
    println!("Common chats: {}", info.common_chats_count);
    println!("Stars rating: {:?}", info.stars_rating);
}
```

### Send a message with all parameters

```rust
client.invoke(&functions::messages::SendMessage {
    no_webpage:        false,
    silent:            false,
    background:        false,
    clear_draft:       true,
    noforwards:        false,
    update_stickersets_order: false,
    invert_media:      false,
    peer:              peer_input,
    reply_to:          None,
    message:           "Hello from raw API!".into(),
    random_id:         ferogram::random_i64_pub(),
    reply_markup:      None,
    entities:          None,
    schedule_date:     None,
    send_as:           None,
    quick_reply_shortcut: None,
    effect:            None,
    allow_paid_floodskip: false,
}).await?;
```

### Edit admin rights (Layer 223)

In Layer 223, `rank` is now `Option<String>`:

```rust
client.invoke(&functions::channels::EditAdmin {
    flags: 0,
    channel: enums::InputChannel::InputChannel(types::InputChannel {
        channel_id, access_hash: ch_hash,
    }),
    user_id: enums::InputUser::InputUser(types::InputUser {
        user_id, access_hash: user_hash,
    }),
    admin_rights: enums::ChatAdminRights::ChatAdminRights(types::ChatAdminRights {
        change_info: true,
        post_messages: true,
        delete_messages: true,
        ban_users: true,
        invite_users: true,
        pin_messages: true,
        manage_call: true,
        manage_ranks: true,  // new in Layer 223
        // ... all others false
        edit_messages: false, add_admins: false, anonymous: false,
        other: false, manage_topics: false, post_stories: false,
        edit_stories: false, delete_stories: false,
        manage_direct_messages: false,
    }),
    rank: Some("Moderator".into()),  // Layer 223: optional
}).await?;
```

### Set bot commands

```rust
client.invoke(&functions::bots::SetBotCommands {
    scope:    enums::BotCommandScope::Default,
    lang_code: "en".into(),
    commands: vec![
        types::BotCommand { command: "start".into(), description: "Start the bot".into() },
        types::BotCommand { command: "help".into(),  description: "Show help".into()  },
        types::BotCommand { command: "ping".into(),  description: "Latency check".into() },
    ],
}).await?;
```

### New in Layer 223: edit chat creator

```rust
client.invoke(&functions::messages::EditChatCreator {
    peer: chat_input_peer,
    user_id: new_creator_input_user,
    password: enums::InputCheckPasswordSRP::InputCheckPasswordEmpty,
}).await?;
```

### New in Layer 223: URL auth match code

```rust
let valid = client.invoke(&functions::messages::CheckUrlAuthMatchCode {
    url:        "https://example.com/login".into(),
    match_code: "abc123".into(),
}).await?;
```

## Access hashes

Many raw API calls need an `access_hash` alongside user/channel IDs. The internal peer cache is populated by `resolve_peer`, `get_participants`, `get_dialogs`, etc.:

```rust
// This populates the peer cache
let peer = client.resolve_peer("@username").await?;

// For users
let user_hash = client.inner_peer_cache_users().get(&user_id).copied().unwrap_or(0);

// Simpler: use resolve_to_input_peer for a ready-to-use InputPeer
let input_peer = client.resolve_to_input_peer("@username").await?;
```

## Error patterns

```rust
use ferogram::{InvocationError, RpcError};

match client.invoke(&req).await {
    Ok(result) => use_result(result),
    Err(InvocationError::Rpc(RpcError { code: 400, message, .. })) => {
        eprintln!("Bad request: {message}");
    }
    Err(InvocationError::Rpc(RpcError { code: 403, message, .. })) => {
        eprintln!("Forbidden: {message}");
    }
    Err(InvocationError::Rpc(RpcError { code: 420, message, .. })) => {
        // FLOOD_WAIT (only if using NoRetries policy)
        let secs: u64 = message
            .strip_prefix("FLOOD_WAIT_").and_then(|s| s.parse().ok()).unwrap_or(60);
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }
    Err(e) => return Err(e.into()),
}
```
