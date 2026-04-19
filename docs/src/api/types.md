# Types Reference

ferogram wraps the raw TL layer's `tl::enums::User` and `tl::enums::Chat`
variants in typed structs so you never need to pattern-match bare enums.

| Wrapper | Underlying TL type |
|---|---|
| `User` | `tl::enums::User` (variant `User`) |
| `Group` | `tl::types::Chat` |
| `Channel` | `tl::types::Channel` |
| `Chat` | Unified enum: `Chat::Group` or `Chat::Channel` |

All four types are available without any feature flags.

---

## User

`User` wraps a non-empty `tl::enums::User::User` variant.

### Construction

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">User::from_raw(raw: tl::enums::User) → Option&lt;User&gt;</span>
</div>
<div class="api-card-body">
Returns <code>None</code> for <code>tl::enums::User::Empty</code>, <code>Some(User)</code> otherwise.
The <code>raw</code> field is public if you need direct TL access.
</div>
</div>

### Identity

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.id() → i64</span>
</div>
<div class="api-card-body">Telegram user ID. Stable and unique forever.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.access_hash() → Option&lt;i64&gt;</span>
</div>
<div class="api-card-body">Access hash needed for most API calls targeting this user. May be <code>None</code> for users not in your contact list or not recently seen.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.first_name() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">First name, if set.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.last_name() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Last name, if set.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.full_name() → String</span>
</div>
<div class="api-card-body">Concatenates first + last name with a space. Returns an empty string if both are absent.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.username() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Primary username without the <code>@</code> prefix.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.usernames() → Vec&lt;&str&gt;</span>
</div>
<div class="api-card-body">All active usernames (primary + extras for Fragment usernames), without <code>@</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.phone() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Phone number, if visible to the logged-in account.</div>
</div>

### Flags

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.bot() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if this is a bot account.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.verified() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the account has a blue verification badge.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.premium() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the user has Telegram Premium.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.is_self() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if this is the currently logged-in account.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.deleted() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the account has been deleted.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.scam() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if Telegram has flagged this account as a scam.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.restricted() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the account is spam-restricted.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.contact() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if this user is in the logged-in user's contact list.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.mutual_contact() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the logged-in user is also in <em>this</em> user's contact list.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.support() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if this account belongs to Telegram support staff.</div>
</div>

### Online & Media

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.status() → Option&lt;&tl::enums::UserStatus&gt;</span>
</div>
<div class="api-card-body">Current online status (<code>Online</code>, <code>Offline</code>, <code>Recently</code>, <code>LastWeek</code>, <code>LastMonth</code>, <code>Empty</code>).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.photo() → Option&lt;&tl::types::UserProfilePhoto&gt;</span>
</div>
<div class="api-card-body">Profile photo metadata, if set. Use <code>client.iter_profile_photos()</code> to download the actual image.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.lang_code() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Language code reported by the user's Telegram client.</div>
</div>

### Bot-specific

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.bot_inline_placeholder() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Placeholder text shown in the compose bar when the user activates this bot's inline mode.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.bot_inline_geo() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the bot can be used inline without requiring a location share.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.bot_supports_chats() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the bot can be added to groups/channels.</div>
</div>

### Peer Conversion

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.as_peer() → tl::enums::Peer</span>
</div>
<div class="api-card-body">Convert to a <code>PeerUser</code> for use in API calls.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">user.as_input_peer() → tl::enums::InputPeer</span>
</div>
<div class="api-card-body">
Convert to an <code>InputPeer</code>. Returns <code>InputPeerUser { user_id, access_hash }</code> when
an access hash is available, or <code>InputPeerPeerSelf</code> for the logged-in account.
</div>
</div>

`User` also implements `Display` as `"Full Name (@username)"` or `"Full Name [id]"`.

---

## Group

`Group` wraps a basic group (`tl::types::Chat`). Basic groups have ≤ 200 members;
larger groups are supergroups (use `Channel` with `megagroup() == true`).

### Construction

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Group::from_raw(raw: tl::enums::Chat) → Option&lt;Group&gt;</span>
</div>
<div class="api-card-body">
Returns <code>None</code> if the raw value is <code>Empty</code>, <code>Forbidden</code>, <code>Channel</code>, or <code>ChannelForbidden</code>.
The <code>raw</code> field (<code>tl::types::Chat</code>) is public for direct TL access.
</div>
</div>

### Accessors

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.id() → i64</span>
</div>
<div class="api-card-body">Group ID.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.title() → &str</span>
</div>
<div class="api-card-body">Group title.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.participants_count() → i32</span>
</div>
<div class="api-card-body">Current member count.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.creator() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the logged-in user is the creator of this group.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.migrated_to() → Option&lt;&tl::enums::InputChannel&gt;</span>
</div>
<div class="api-card-body">If the group was upgraded to a supergroup, contains the <code>InputChannel</code> of the new supergroup.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.as_peer() → tl::enums::Peer</span>
</div>
<div class="api-card-body">Convert to <code>PeerChat</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">group.as_input_peer() → tl::enums::InputPeer</span>
</div>
<div class="api-card-body">Convert to <code>InputPeerChat</code>.</div>
</div>

`Group` implements `Display` as `"Title [group id]"`.

---

## Channel

`Channel` wraps both broadcast channels and supergroups
(`tl::types::Channel`). Use `kind()` or `megagroup()` / `broadcast()` to
distinguish them.

### Construction

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Channel::from_raw(raw: tl::enums::Chat) → Option&lt;Channel&gt;</span>
</div>
<div class="api-card-body">
Returns <code>None</code> for non-channel variants. The <code>raw</code> field (<code>tl::types::Channel</code>) is public.
</div>
</div>

### Identity

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.id() → i64</span>
</div>
<div class="api-card-body">Channel / supergroup ID.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.access_hash() → Option&lt;i64&gt;</span>
</div>
<div class="api-card-body">Access hash required for channel-targeted API calls.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.title() → &str</span>
</div>
<div class="api-card-body">Channel / supergroup title.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.username() → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Primary public username without <code>@</code>, if set.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.usernames() → Vec&lt;&str&gt;</span>
</div>
<div class="api-card-body">All active usernames (primary + Fragment extras), without <code>@</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.participants_count() → Option&lt;i32&gt;</span>
</div>
<div class="api-card-body">Approximate member count. May be <code>None</code> for private channels.</div>
</div>

### Kind

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.kind() → ChannelKind</span>
</div>
<div class="api-card-body">

| Variant | Description |
|---|---|
| `ChannelKind::Broadcast` | Broadcast channel (posts only) |
| `ChannelKind::Megagroup` | Supergroup (all members can post) |
| `ChannelKind::Gigagroup` | Large broadcast group (gigagroup) |

</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.megagroup() → bool</span>
</div>
<div class="api-card-body"><code>true</code> for supergroups.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.broadcast() → bool</span>
</div>
<div class="api-card-body"><code>true</code> for broadcast channels.</div>
</div>

### Flags

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.verified() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the channel has a verification badge.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.restricted() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if the channel is unavailable in certain regions.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.signatures() → bool</span>
</div>
<div class="api-card-body"><code>true</code> if post author signatures are shown in the channel.</div>
</div>

### Rights & Media

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.admin_rights() → Option&lt;&tl::types::ChatAdminRights&gt;</span>
</div>
<div class="api-card-body">Admin rights granted to the logged-in user in this channel, if any.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.photo() → Option&lt;&tl::types::ChatPhoto&gt;</span>
</div>
<div class="api-card-body">Channel profile photo metadata, if set.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.restriction_reason() → Vec&lt;&tl::enums::RestrictionReason&gt;</span>
</div>
<div class="api-card-body">Regional restriction reasons (e.g. country codes where the channel is blocked).</div>
</div>

### Peer Conversion

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.as_peer() → tl::enums::Peer</span>
</div>
<div class="api-card-body">Convert to <code>PeerChannel</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.as_input_peer() → tl::enums::InputPeer</span>
</div>
<div class="api-card-body">Convert to <code>InputPeerChannel { channel_id, access_hash }</code>. Returns <code>InputPeerEmpty</code> if the access hash is absent.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">channel.as_input_channel() → tl::enums::InputChannel</span>
</div>
<div class="api-card-body">Convert to <code>InputChannel</code> for channel-specific RPCs (e.g. <code>channels.GetParticipants</code>). Returns <code>InputChannelEmpty</code> if the access hash is absent.</div>
</div>

`Channel` implements `Display` as `"Title (@username)"` or `"Title [channel id]"`.

---

## Chat (unified enum)

`Chat` is a convenience enum that holds either a `Group` or a `Channel`.
Most client methods return peers as `Chat` when the type is not known in advance.

```rust,no_run
match chat {
    Chat::Group(g) => println!("Basic group: {}", g.title()),
    Chat::Channel(c) => println!("Channel/supergroup: {}", c.title()),
}
```

### Construction

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Chat::from_raw(raw: tl::enums::Chat) → Option&lt;Chat&gt;</span>
</div>
<div class="api-card-body">Returns <code>None</code> for <code>Empty</code>, <code>Forbidden</code>, and <code>ChannelForbidden</code> variants.</div>
</div>

### Common Accessors

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">chat.id() → i64</span>
</div>
<div class="api-card-body">ID regardless of variant.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">chat.title() → &str</span>
</div>
<div class="api-card-body">Title regardless of variant.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">chat.as_peer() → tl::enums::Peer</span>
</div>
<div class="api-card-body">Convert to the appropriate <code>Peer</code> variant.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">chat.as_input_peer() → tl::enums::InputPeer</span>
</div>
<div class="api-card-body">Convert to the appropriate <code>InputPeer</code> variant.</div>
</div>

---

## Quick Reference

```rust,no_run
use ferogram::{User, Group, Channel, Chat};

// User
if let Some(user) = User::from_raw(raw_user) {
    println!("{} id={}", user.full_name(), user.id());
    if user.bot() { println!("It's a bot"); }
    let peer = user.as_input_peer(); // for API calls
}

// Group
if let Some(group) = Group::from_raw(raw_chat) {
    println!("{} ({} members)", group.title(), group.participants_count());
}

// Channel / Supergroup
if let Some(ch) = Channel::from_raw(raw_chat) {
    match ch.kind() {
        ferogram::ChannelKind::Broadcast => println!("Channel"),
        ferogram::ChannelKind::Megagroup => println!("Supergroup"),
        ferogram::ChannelKind::Gigagroup => println!("Gigagroup"),
    }
}

// Unified
if let Some(chat) = Chat::from_raw(raw_chat) {
    println!("id={} title={}", chat.id(), chat.title());
}
```
