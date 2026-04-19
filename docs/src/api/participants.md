# Participants & Members

Methods for fetching, banning, kicking, promoting, and managing chat members. All methods accept `impl Into<PeerRef>` for the peer argument.

---

## Fetch participants

```rust
use ferogram::participants::Participant;

// Fetch up to N participants at once
let members: Vec<Participant> = client
    .get_participants(peer.clone(), 200)
    .await?;

for p in &members {
    println!(
        "{}: admin: {}, banned: {}",
        p.user.first_name.as_deref().unwrap_or("?"),
        p.is_admin(),
        p.is_banned(),
    );
}
```

### Paginated iterator (large groups)

```rust
let members = client.iter_participants(peer.clone(), None, 0).await?;
for p in &members {
    println!("{}", p.user.first_name.as_deref().unwrap_or(""));
}
```

### Search contacts and dialogs by name

```rust
// Returns combined results from contacts, dialogs, and global
let results: Vec<tl::enums::Peer> = client.search_peer("John").await?;
```

---

## `Participant` fields

```rust
p.user          // tl::types::User: raw user data
p.is_creator()  // bool: is the channel/group creator
p.is_admin()    // bool: has any admin rights
p.is_banned()   // bool: is banned/restricted
p.is_member()   // bool: active member (not banned, not left)
```

---

## Kick participant

```rust
// Removes the user from a basic group by chat_id (i64)
// For channels/supergroups, use ban_participant instead
client.kick_participant(chat_id, user_id).await?;
```

> **Note:** `kick_participant` takes a `chat_id: i64`, not a `PeerRef`. Use the raw numeric ID of the basic group.

---

## Ban participant

`ban_participant` takes a Unix timestamp for `until_date`. For granular per-permission restrictions use `set_banned_rights` with `BannedRightsBuilder`.

```rust
// Permanent full ban (until_date = 0)
client.ban_participant(peer.clone(), user_id, 0).await?;

// Timed ban: expires in 24 h
let expires = (std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400) as i32;
client.ban_participant(peer.clone(), user_id, expires).await?;

// Unban: set a past timestamp (e.g. 1) to lift the ban
client.ban_participant(peer.clone(), user_id, 1).await?;
```

For per-permission restrictions (no media, no stickers, etc.) use `set_banned_rights` with `BannedRightsBuilder`:

```rust
use ferogram::participants::BannedRightsBuilder;

client
    .set_banned_rights(
        peer.clone(),
        user_input_peer,
        BannedRightsBuilder::new()
            .send_media(true)
            .send_stickers(true)
            .until_date(expires),
    )
    .await?;
```

### `BannedRightsBuilder` methods

| Method | Description |
|---|---|
| `BannedRightsBuilder::new()` | All permissions granted (empty ban = unban) |
| `BannedRightsBuilder::full_ban()` | All rights revoked, permanent |
| `.view_messages(bool)` | Prevent reading messages |
| `.send_messages(bool)` | Prevent sending text |
| `.send_media(bool)` | Prevent sending media |
| `.send_stickers(bool)` | Prevent sending stickers |
| `.send_gifs(bool)` | Prevent sending GIFs |
| `.send_games(bool)` | Prevent sending games |
| `.send_inline(bool)` | Prevent using inline bots |
| `.embed_links(bool)` | Prevent embedding links |
| `.send_polls(bool)` | Prevent sending polls |
| `.change_info(bool)` | Prevent changing chat info |
| `.invite_users(bool)` | Prevent inviting users |
| `.pin_messages(bool)` | Prevent pinning messages |
| `.until_date(ts: i32)` | Expiry Unix timestamp (`0` = permanent) |

---

## Promote admin: `promote_participant` and `AdminRightsBuilder`

`promote_participant` is a boolean shorthand  -  `true` grants all standard rights, `false` demotes:

```rust
// Quick promote (all standard rights except add_admins)
client.promote_participant(peer.clone(), user_id, true).await?;

// Demote back to regular member
client.promote_participant(peer.clone(), user_id, false).await?;
```

For fine-grained control, use `set_admin_rights` with `AdminRightsBuilder`:

```rust
use ferogram::participants::AdminRightsBuilder;

// Promote with specific rights and a custom title
client
    .promote_participant(
        peer.clone(),
        user_id,
        AdminRightsBuilder::new()
            .post_messages(true)
            .delete_messages(true)
            .ban_users(true)
            .invite_users(true)
            .pin_messages(true)
            .rank("Moderator"), // custom admin title (max 16 chars)
    )
    .await?;

// Full admin (all standard rights except add_admins)
client
    .promote_participant(peer.clone(), user_id, AdminRightsBuilder::full_admin())
    .await?;

// Demote: pass an empty builder to remove all admin rights
client
    .promote_participant(peer.clone(), user_id, AdminRightsBuilder::new())
    .await?;
```

### `AdminRightsBuilder` methods

| Method | Description |
|---|---|
| `AdminRightsBuilder::new()` | No rights (use to demote) |
| `AdminRightsBuilder::full_admin()` | All standard rights |
| `.change_info(bool)` | Can change channel/group info |
| `.post_messages(bool)` | Can post in channels |
| `.edit_messages(bool)` | Can edit others' messages |
| `.delete_messages(bool)` | Can delete messages |
| `.ban_users(bool)` | Can ban / restrict members |
| `.invite_users(bool)` | Can add members |
| `.pin_messages(bool)` | Can pin messages |
| `.add_admins(bool)` | Can promote other admins (**use carefully**) |
| `.anonymous(bool)` | Posts appear as channel name, not user |
| `.manage_call(bool)` | Can manage voice/video chats |
| `.manage_topics(bool)` | Can manage forum topics |
| `.rank(str)` | Custom admin title shown beside name |

---

## Get participant permissions

Check the effective permissions of a user in a channel or supergroup:

```rust
use ferogram::participants::ParticipantPermissions;

let perms: ParticipantPermissions = client
    .get_permissions(peer.clone(), user_id)
    .await?;

println!("Creator: {}", perms.is_creator());
println!("Admin: {}",   perms.is_admin());
println!("Banned: {}",  perms.is_banned());
println!("Member: {}",  perms.is_member());
println!("Can send: {}", perms.can_send_messages);
println!("Can pin: {}",  perms.can_pin_messages);
println!("Admin title: {:?}", perms.admin_rank);
```

### `ParticipantPermissions` fields & methods

| Symbol | Type | Description |
|---|---|---|
| `is_creator()` | `bool` | Is the channel/group creator |
| `is_admin()` | `bool` | Has any admin rights |
| `is_banned()` | `bool` | Is banned or restricted |
| `is_member()` | `bool` | Active member (not banned, not left) |
| `can_send_messages` | `bool` | Can send text messages |
| `can_send_media` | `bool` | Can send media |
| `can_pin_messages` | `bool` | Can pin messages |
| `can_add_admins` | `bool` | Can promote admins |
| `admin_rank` | `Option<String>` | Custom admin title |

---

## Profile photos

```rust
// Fetch a page of profile photos (limit = number to fetch)
let photos = client.get_profile_photos(peer.clone(), 10).await?;

// Lazy iterator across all pages (chunk_size = 0 uses default of 100)
let mut iter = client.iter_profile_photos(peer.clone(), 0).await?;
while let Some(photo) = iter.next().await? {
    // download photo bytes...
}
```

---

## Join and leave chats

```rust
// Join a public group or channel
client.join_chat("@somegroup").await?;

// Accept a private invite link
client.accept_invite_link("https://t.me/joinchat/AbCdEfG").await?;

// Parse invite hash from any link format
let hash = Client::parse_invite_hash("https://t.me/+AbCdEfG12345");

// Leave / remove dialog
client.delete_dialog(peer.clone()).await?;
```

---

## `ParticipantStatus` enum

```rust
use ferogram::participants::ParticipantStatus;

for p in &members {
    match p.status {
        ParticipantStatus::Member     => println!("Regular member"),
        ParticipantStatus::Creator    => println!("Creator"),
        ParticipantStatus::Admin      => println!("Admin"),
        ParticipantStatus::Restricted => println!("Restricted"),
        ParticipantStatus::Banned     => println!("Banned"),
        ParticipantStatus::Left       => println!("Left"),
    }
}
```

---

## `ProfilePhotoIter`: extended methods

```rust
let mut iter = client.iter_profile_photos(user_id);

// Total photo count (available after first fetch)
if let Some(total) = iter.total_count() {
    println!("{total} profile photos");
}

// Collect all photos at once
let all_photos = iter.collect().await?;
```

| Method | Description |
|---|---|
| `iter.next()` | `async → Option<tl::enums::Photo>` |
| `iter.collect()` | `async → Vec<tl::enums::Photo>`: all photos |
| `iter.total_count()` | `Option<i32>`: total count after first fetch |

---

## Low-level rights setters

For advanced use cases, `set_banned_rights` and `set_admin_rights` give direct access to the TL layer:

```rust
// Set banned rights directly (channel/supergroup only)
client.set_banned_rights(
    peer.clone(),
    user_input_peer,
    BannedRightsBuilder::new().send_media(true),
).await?;

// Set admin rights directly
client.set_admin_rights(
    peer.clone(),
    user_input_peer,
    AdminRightsBuilder::new().delete_messages(true),
    Some("Moderator".into()),  // optional custom rank
).await?;
```
