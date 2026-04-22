# Chat Management

Full reference for creating, editing, and deleting groups and channels. All methods are `async` and return `Result<_, InvocationError>`.

For invite link management see [Invite Links](./invite-links.md). For forum topics see [Forum Topics](./forum-topics.md).

---

## Join & leave

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.join_chat(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Join a public group or channel by peer reference (username, ID, or t.me link).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.accept_invite_link(link: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Accept a <code>t.me/+hash</code> or <code>t.me/joinchat/hash</code> invite link and join the chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.leave_chat(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Leave a channel or supergroup. For basic groups, use <code>kick_participant</code> on yourself or <code>delete_dialog</code> to hide it.</div>
</div>

---

## Create

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.create_group(title: impl Into&lt;String&gt;, user_ids: Vec&lt;i64&gt;) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Create a new legacy basic group with an initial member list. Basic groups support up to 200 members. To go larger, call <code>migrate_chat</code> to upgrade to a supergroup.

```rust
let chat = client.create_group("Dev Team", vec![user_a, user_b]).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.create_channel(title: impl Into&lt;String&gt;, about: impl Into&lt;String&gt;, broadcast: bool) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Create a new channel (<code>broadcast = true</code>) or supergroup (<code>broadcast = false</code>).

```rust
// Supergroup
let sg = client.create_channel("My Community", "A place to chat", false).await?;

// Broadcast channel
let ch = client.create_channel("My News", "Daily updates", true).await?;
```
</div>
</div>

---

## Delete & migrate

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_channel(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Permanently delete a channel or supergroup. Only the creator can do this. Irreversible  -  all messages are lost.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_chat(chat_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Delete a legacy basic group by its raw numeric chat ID. Only the creator can do this. For supergroups and channels use <code>delete_channel</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.migrate_chat(chat_id: i64) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Upgrade a legacy basic group to a supergroup. Returns the new channel peer. The original <code>chat_id</code> becomes invalid after migration; update any stored references.</div>
</div>

---

## Edit

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_title(peer: impl Into&lt;PeerRef&gt;, title: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Rename a chat, group, channel, or supergroup.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_about(peer: impl Into&lt;PeerRef&gt;, about: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set or update the description/about text. Works for all chat types.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_photo(peer: impl Into&lt;PeerRef&gt;, photo: tl::enums::InputChatPhoto) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Change the group/channel photo. Pass <code>InputChatPhoto::Empty</code> to remove it.

```rust
// Set a new photo (upload first)
let uploaded = client.upload_file(&bytes, "photo.jpg", "image/jpeg").await?;
client.edit_chat_photo(
    peer.clone(),
    tl::enums::InputChatPhoto::InputChatUploadedPhoto(
        tl::types::InputChatUploadedPhoto {
            video: false, video_emoji_markup: None,
            file: Some(uploaded.into_input_file()),
        }
    ),
).await?;

// Remove photo
client.edit_chat_photo(peer.clone(), tl::enums::InputChatPhoto::Empty).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_default_banned_rights(peer: impl Into&lt;PeerRef&gt;, build: impl FnOnce(BannedRightsBuilder) → BannedRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set default permissions for <em>all</em> members via a closure on <code>BannedRightsBuilder</code>. Passing <code>true</code> to a method restricts that action.

```rust
// Read-only group: members can only read
client.edit_chat_default_banned_rights(peer.clone(), |b| {
    b.send_messages(true)
     .send_media(true)
     .send_polls(true)
}).await?;

// Restore all defaults
client.edit_chat_default_banned_rights(peer.clone(), |b| b).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_chat_theme(peer: impl Into&lt;PeerRef&gt;, emoticon: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set the emoji colour theme for a chat. Pass a single emoji (e.g. <code>"🌈"</code>) to apply it, or an empty string to reset to the default.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_chat_reactions(peer: impl Into&lt;PeerRef&gt;, reactions: tl::enums::ChatReactions) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Control which reactions members can use. See <a href="../api/client.md#chat-management">Client Methods § Chat management</a> for the three variant forms.</div>
</div>

---

## Members & info

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.invite_users(peer: impl Into&lt;PeerRef&gt;, user_ids: Vec&lt;i64&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Add one or more users to a chat. For channels all users are added in a single request; for basic groups each user is added individually.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_chat_full(peer: impl Into&lt;PeerRef&gt;) → Result&lt;tl::enums::messages::ChatFull, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the full info object for any chat. Contains description, pinned message ID, linked channel, member count, slow mode delay, call info, and more.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_online_count(peer: impl Into&lt;PeerRef&gt;) → Result&lt;i32, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the approximate number of members currently online in a group or channel.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_common_chats(user_id: i64, max_id: i64, limit: i32) → Result&lt;Vec&lt;tl::enums::Chat&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get chats shared between the current account and <code>user_id</code>. Start with <code>max_id = 0</code>; use the last returned chat ID for subsequent pages. Max <code>limit</code> is 100.</div>
</div>

---

## Moderation settings

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_no_forwards(peer: impl Into&lt;PeerRef&gt;, enabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Enable or disable the no-forwards restriction. When on, members cannot forward messages out of this chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_history_ttl(peer: impl Into&lt;PeerRef&gt;, period: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set the auto-delete timer for messages. <code>period</code> is in seconds  -  common values: <code>86400</code> (1 day), <code>604800</code> (1 week), <code>2678400</code> (1 month). Pass <code>0</code> to disable.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_forum(peer: impl Into&lt;PeerRef&gt;, enabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Enable or disable forum (topics) mode on a supergroup. See <a href="./forum-topics.md">Forum Topics</a> for full topic management after enabling.</div>
</div>

---

## Quick-start recipe

```rust
// 1. Create a supergroup
let chat = client.create_channel("My Group", "Welcome!", false).await?;

// 2. Add members
client.invite_users(chat.clone(), vec![user_a, user_b]).await?;

// 3. Set description and lock down media for all
client.edit_chat_about(chat.clone(), "Read the rules before posting.").await?;
client.edit_chat_default_banned_rights(chat.clone(), |b| {
    b.send_media(true).send_polls(true)
}).await?;

// 4. Enable auto-delete (1 week)
client.set_history_ttl(chat.clone(), 604_800).await?;

// 5. Generate an approval-gated invite link
let inv = client.export_invite_link(chat.clone(), None, None, true).await?;
println!("Invite: {}", match &inv {
    tl::enums::ExportedChatInvite::Invite(i) => &i.link,
    _ => "",
});
```

---

## Transfer ownership

Transfer ownership of a basic group to another member. The calling user must be the current owner and must supply their 2FA SRP credential.

```rust
use ferogram_tl_types as tl;

// For a no-password account, use InputCheckPasswordEmpty
let password_check = tl::enums::InputCheckPasswordSrp::Empty(
    tl::types::InputCheckPasswordEmpty {}
);

client.transfer_chat_ownership(
    "@mygroup",
    new_owner_user_id,
    password_check,
).await?;
```

Use [`Client::compute_password_check`](./profile.md) to build the SRP object when 2FA is enabled.

> **Note**: For channels and supergroups, ownership transfer uses `channels.editCreator` on the Telegram layer, which is not yet wrapped by a dedicated helper. Use the [raw API](../advanced/raw-api.md) for that case.

---

## Linked channel

A broadcast channel can have a linked discussion supergroup and vice-versa. Retrieve the linked chat's ID:

```rust
if let Some(linked_id) = client.get_linked_channel("@mychannel").await? {
    println!("Linked chat ID: {linked_id}");
}
```

Returns `None` when no linked chat is configured. Works for both directions - pass a channel to get its discussion group, or pass a supergroup to get its linked broadcast channel.
