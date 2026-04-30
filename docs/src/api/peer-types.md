# Peer Types & PeerRef

`ferogram` provides typed wrappers over the raw `tl::enums::User` and `tl::enums::Chat` types, plus `PeerRef`: a flexible peer argument accepted by every `Client` method.

---

## `PeerRef`: flexible peer argument

Every `Client` method that previously required a bare `tl::enums::Peer` now accepts `impl Into<PeerRef>`. That means you can pass:

```rust
// @username string (with or without @)
client.send_message("@durov", "hi").await?;
client.send_message("durov",  "hi").await?;

// "me" / "self": always resolves to the logged-in account
client.send_message("me", "Note to self").await?;

// Positive i64: Telegram user ID
client.send_message(12345678_i64, "hi").await?;

// Negative i64: Bot-API channel ID (-100… prefix)
client.iter_messages(-1001234567890_i64);

// Negative i64: Bot-API basic-group ID (small negative)
client.mark_as_read(-123456_i64).await?;

// Already-resolved TL peer: zero overhead, no network call
use ferogram::tl;
let peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 123 });
client.send_message(peer, "hi").await?;
```

### `PeerRef` variants

| Variant | How resolved |
|---|---|
| `PeerRef::Username(s)` | `contacts.resolveUsername` RPC (cached after first call) |
| `PeerRef::Id(i64)` | Decoded from Bot-API encoding: **no network call** |
| `PeerRef::Peer(tl::enums::Peer)` | Forwarded as-is: **zero cost** |

### Bot-API ID encoding

| Range | Maps to |
|---|---|
| `id > 0` | User (`PeerUser { user_id: id }`) |
| `-1_000_000_000_000 < id < 0` | Basic group (`PeerChat { chat_id: -id }`) |
| `id ≤ -1_000_000_000_000` | Channel (`channel_id = -id - 1_000_000_000_000`) |

### Resolving manually

```rust
use ferogram::PeerRef;

let peer_ref = PeerRef::from("@someuser");
let peer: tl::enums::Peer = peer_ref.resolve(&client).await?;
```

---

## `User`: user account wrapper

```rust
use ferogram::types::User;

// Wrap from raw TL
if let Some(user) = User::from_raw(raw_tl_user) {
    println!("ID: {}", user.id());
    println!("Name: {}", user.full_name());
    println!("Username: {:?}", user.username());
    println!("Is bot: {}", user.bot());
    println!("Is premium: {}", user.premium());
}
```

### `User` accessor methods

| Method | Return type | Description |
|---|---|---|
| `id()` | `i64` | Telegram user ID |
| `access_hash()` | `Option<i64>` | Access hash for API calls |
| `first_name()` | `Option<&str>` | First name |
| `last_name()` | `Option<&str>` | Last name |
| `full_name()` | `String` | `"First [Last]"` combined |
| `username()` | `Option<&str>` | Primary username (without `@`) |
| `usernames()` | `Vec<&str>` | All active usernames |
| `phone()` | `Option<&str>` | Phone number (if visible) |
| `bot()` | `bool` | Is a bot account |
| `verified()` | `bool` | Is a verified account |
| `premium()` | `bool` | Is a premium account |
| `deleted()` | `bool` | Account has been deleted |
| `scam()` | `bool` | Flagged as scam |
| `restricted()` | `bool` | Account is restricted |
| `is_self()` | `bool` | Is the currently logged-in user |
| `contact()` | `bool` | In the logged-in user's contacts |
| `mutual_contact()` | `bool` | Mutual contact |
| `support()` | `bool` | Telegram support staff |
| `lang_code()` | `Option<&str>` | User's client language code |
| `status()` | `Option<&tl::enums::UserStatus>` | Online/offline status |
| `photo()` | `Option<&tl::types::UserProfilePhoto>` | Profile photo |
| `bot_inline_placeholder()` | `Option<&str>` | Inline mode compose bar hint |
| `bot_inline_geo()` | `bool` | Bot supports inline without location |
| `bot_supports_chats()` | `bool` | Bot can be added to groups |
| `restriction_reason()` | `Vec<&tl::enums::RestrictionReason>` | Restriction reasons |
| `as_peer()` | `tl::enums::Peer` | Convert to `Peer` |
| `as_input_peer()` | `tl::enums::InputPeer` | Convert to `InputPeer` |

`User` implements `Display` as `"Full Name (@username)"` or `"Full Name [user_id]"`.

---

## `Group`: basic group wrapper

```rust
use ferogram::types::Group;

if let Some(group) = Group::from_raw(raw_tl_chat) {
    println!("ID: {}", group.id());
    println!("Title: {}", group.title());
    println!("Members: {}", group.participants_count());
    println!("I am creator: {}", group.creator());
}
```

### `Group` accessor methods

| Method | Return type | Description |
|---|---|---|
| `id()` | `i64` | Group ID |
| `title()` | `&str` | Group name |
| `participants_count()` | `i32` | Member count |
| `creator()` | `bool` | Logged-in user is the creator |
| `migrated_to()` | `Option<&tl::enums::InputChannel>` | Points to supergroup after migration |
| `as_peer()` | `tl::enums::Peer` | Convert to `Peer` |
| `as_input_peer()` | `tl::enums::InputPeer` | Convert to `InputPeer` |

---

## `Channel`: channel / supergroup wrapper

```rust
use ferogram::types::{Channel, ChannelKind};

if let Some(channel) = Channel::from_raw(raw_tl_chat) {
    println!("ID: {}", channel.id());
    println!("Title: {}", channel.title());
    println!("Username: {:?}", channel.username());
    println!("Kind: {:?}", channel.kind());
    println!("Members: {:?}", channel.participants_count());
}
```

### `Channel` accessor methods

| Method | Return type | Description |
|---|---|---|
| `id()` | `i64` | Channel ID |
| `access_hash()` | `Option<i64>` | Access hash |
| `title()` | `&str` | Channel / supergroup name |
| `username()` | `Option<&str>` | Public username (without `@`) |
| `usernames()` | `Vec<&str>` | All active usernames |
| `megagroup()` | `bool` | Is a supergroup (not a broadcast channel) |
| `broadcast()` | `bool` | Is a broadcast channel |
| `gigagroup()` | `bool` | Is a broadcast group (gigagroup) |
| `kind()` | `ChannelKind` | `Broadcast` / `Megagroup` / `Gigagroup` |
| `verified()` | `bool` | Verified account |
| `restricted()` | `bool` | Is restricted |
| `signatures()` | `bool` | Posts have author signatures |
| `participants_count()` | `Option<i32>` | Approximate member count |
| `photo()` | `Option<&tl::types::ChatPhoto>` | Channel photo |
| `admin_rights()` | `Option<&tl::types::ChatAdminRights>` | Your admin rights |
| `restriction_reason()` | `Vec<&tl::enums::RestrictionReason>` | Restriction reasons |
| `as_peer()` | `tl::enums::Peer` | Convert to `Peer` |
| `as_input_peer()` | `tl::enums::InputPeer` | Convert to `InputPeer` (requires hash) |
| `as_input_channel()` | `tl::enums::InputChannel` | Convert to `InputChannel` |

### `ChannelKind` enum

```rust
use ferogram::types::ChannelKind;

match channel.kind() {
    ChannelKind::Broadcast  => { /* Posts only, no member replies */ }
    ChannelKind::Megagroup  => { /* All members can post */ }
    ChannelKind::Gigagroup  => { /* Large public broadcast group */ }
}
```

---

## `Chat`: unified chat enum

`Chat` unifies `Group` and `Channel` into one enum with shared accessors:

```rust
use ferogram::types::Chat;

if let Some(chat) = Chat::from_raw(raw_tl_chat) {
    println!("ID: {}", chat.id());
    println!("Title: {}", chat.title());

    match &chat {
        Chat::Group(g)   => println!("Basic group, {} members", g.participants_count()),
        Chat::Channel(c) => println!("{:?} channel", c.kind()),
    }
}
```

### `Chat` methods

| Method | Return type | Description |
|---|---|---|
| `id()` | `i64` | ID regardless of variant |
| `title()` | `&str` | Name regardless of variant |
| `as_peer()` | `tl::enums::Peer` | `Peer` variant |
| `as_input_peer()` | `tl::enums::InputPeer` | `InputPeer` variant |
