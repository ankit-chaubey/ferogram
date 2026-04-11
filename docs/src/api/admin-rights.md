# Admin & Ban Rights

`ferogram` provides two fluent builders for granular admin and ban rights: `AdminRightsBuilder` and `BannedRightsBuilder`: both in the `ferogram::participants` module.

---

## `BannedRightsBuilder`: restrict a member

```rust
use ferogram::participants::BannedRightsBuilder;

// Permanent full ban
client
    .ban_participant(peer.clone(), user_id, BannedRightsBuilder::full_ban())
    .await?;

// Partial restriction (no media, expires in 24 h)
let tomorrow = (std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400) as i32;

client
    .ban_participant(
        peer.clone(), user_id,
        BannedRightsBuilder::new()
            .send_media(true)
            .send_stickers(true)
            .send_gifs(true)
            .until_date(tomorrow),
    )
    .await?;

// Unban
// Passing an empty builder restores full permissions
client
    .ban_participant(peer.clone(), user_id, BannedRightsBuilder::new())
    .await?;
```

### Method reference

| Method | Default | Description |
|---|---|---|
| `BannedRightsBuilder::new()` | all `false` | No restrictions (use to unban) |
| `BannedRightsBuilder::full_ban()` | all `true`, `until_date = 0` | Total permanent ban |
| `.view_messages(bool)` | `false` | Block reading messages |
| `.send_messages(bool)` | `false` | Block sending text |
| `.send_media(bool)` | `false` | Block sending media |
| `.send_stickers(bool)` | `false` | Block stickers |
| `.send_gifs(bool)` | `false` | Block GIFs |
| `.send_games(bool)` | `false` | Block games |
| `.send_inline(bool)` | `false` | Block inline bots |
| `.embed_links(bool)` | `false` | Block link embeds |
| `.send_polls(bool)` | `false` | Block polls |
| `.change_info(bool)` | `false` | Block changing chat info |
| `.invite_users(bool)` | `false` | Block inviting users |
| `.pin_messages(bool)` | `false` | Block pinning messages |
| `.until_date(ts: i32)` | `0` | Expiry Unix timestamp (`0` = permanent) |

> **Note:** Setting `view_messages: true` is a full ban: the member cannot read messages or remain in the group.

---

## `AdminRightsBuilder`: grant admin rights

```rust
use ferogram::participants::AdminRightsBuilder;

// Custom moderator
client
    .promote_participant(
        peer.clone(), user_id,
        AdminRightsBuilder::new()
            .delete_messages(true)
            .ban_users(true)
            .invite_users(true)
            .pin_messages(true)
            .rank("Moderator"),  // shown next to name
    )
    .await?;

// Full admin (all standard rights)
client
    .promote_participant(peer.clone(), user_id, AdminRightsBuilder::full_admin())
    .await?;

// Demote (remove all admin rights)
client
    .promote_participant(peer.clone(), user_id, AdminRightsBuilder::new())
    .await?;
```

### Method reference

| Method | Default | Description |
|---|---|---|
| `AdminRightsBuilder::new()` | all `false` | No rights (use to demote) |
| `AdminRightsBuilder::full_admin()` | standard set | All rights except `add_admins` |
| `.change_info(bool)` | `false` | Can edit channel/group info & photo |
| `.post_messages(bool)` | `false` | Can post in broadcast channels |
| `.edit_messages(bool)` | `false` | Can edit other users' messages |
| `.delete_messages(bool)` | `false` | Can delete any message |
| `.ban_users(bool)` | `false` | Can restrict / ban members |
| `.invite_users(bool)` | `false` | Can add new members |
| `.pin_messages(bool)` | `false` | Can pin messages |
| `.add_admins(bool)` | `false` | Can promote others to admin ⚠️ |
| `.anonymous(bool)` | `false` | Posts appear as channel name |
| `.manage_call(bool)` | `false` | Can start/manage voice chats |
| `.manage_topics(bool)` | `false` | Can create/edit/delete forum topics |
| `.rank(str)` | `None` | Custom admin title (max 16 chars) |

> `.add_admins(true)` grants significant trust: admins with this right can promote others to full admin level.

---

## `ParticipantPermissions`: read effective rights

To inspect the actual current permissions of a user in a channel:

```rust
use ferogram::participants::ParticipantPermissions;

let perms: ParticipantPermissions = client
    .get_permissions(peer.clone(), user_id)
    .await?;
```

### Fields & methods

| Symbol | Type | Description |
|---|---|---|
| `is_creator()` | `bool` | Is the creator |
| `is_admin()` | `bool` | Has any admin rights |
| `is_banned()` | `bool` | Is banned or restricted |
| `is_member()` | `bool` | Active member (`!is_banned && !is_left`) |
| `can_send_messages` | `bool` | Can send text |
| `can_send_media` | `bool` | Can send media |
| `can_pin_messages` | `bool` | Can pin messages |
| `can_add_admins` | `bool` | Can promote others |
| `admin_rank` | `Option<String>` | Custom admin title |

---

## Quick patterns

```rust
// Temporarily mute: no messages for 1 hour
let in_1h = (chrono::Utc::now().timestamp() + 3600) as i32;
client.ban_participant(peer.clone(), uid,
    BannedRightsBuilder::new().send_messages(true).until_date(in_1h)
).await?;

// Promote to channel editor
client.promote_participant(peer.clone(), uid,
    AdminRightsBuilder::new()
        .post_messages(true)
        .edit_messages(true)
        .delete_messages(true)
).await?;

// Check before acting
let perms = client.get_permissions(peer.clone(), uid).await?;
if !perms.is_admin() && !perms.is_banned() {
    client.ban_participant(peer.clone(), uid, BannedRightsBuilder::full_ban()).await?;
}
```
