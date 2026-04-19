# Forum Topics

Telegram supergroups can be converted to **forums**  -  they gain named topics that act as separate message threads inside one chat. Each topic has its own message history, read state, and optional icon.

Enable forum mode on a supergroup with [`toggle_forum`](#enable--disable-forum-mode).

---

## Enable / disable forum mode

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_forum(peer: impl Into&lt;PeerRef&gt;, enabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Turn forum mode on or off for a supergroup. Requires channel admin rights. Once enabled the chat gains a <em>General</em> topic (topic ID 1) automatically.

```rust
client.toggle_forum(peer.clone(), true).await?;   // enable
client.toggle_forum(peer.clone(), false).await?;  // disable
```
</div>
</div>

---

## List topics

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_forum_topics(peer: impl Into&lt;PeerRef&gt;, query: Option&lt;String&gt;, limit: i32, offset_date: i32, offset_id: i32, offset_topic: i32) → Result&lt;Vec&lt;tl::enums::ForumTopic&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Paginate through forum topics. Use <code>query</code> to search by topic title. Start with all offsets at <code>0</code>.

| Param | Description |
|---|---|
| `query` | Search string. `None` returns all topics. |
| `limit` | Topics per page (max 100). |
| `offset_date` | Date of last topic in previous page (for pagination). |
| `offset_id` | `top_msg_id` of last topic in previous page. |
| `offset_topic` | Topic ID of last topic in previous page. |

```rust
// First page
let topics = client
    .get_forum_topics(peer.clone(), None, 100, 0, 0, 0)
    .await?;

for topic in &topics {
    if let tl::enums::ForumTopic::Topic(t) = topic {
        println!("[{}] {}  -  {} unread", t.id, t.title, t.unread_count);
    }
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_forum_topics_by_id(peer: impl Into&lt;PeerRef&gt;, topic_ids: Vec&lt;i32&gt;) → Result&lt;Vec&lt;tl::enums::ForumTopic&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch specific topics by their IDs in one request.

```rust
let topics = client.get_forum_topics_by_id(peer.clone(), vec![1, 42, 99]).await?;
```
</div>
</div>

---

## Create & edit topics

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.create_forum_topic(peer: impl Into&lt;PeerRef&gt;, title: impl Into&lt;String&gt;, icon_color: Option&lt;i32&gt;, icon_emoji_id: Option&lt;i64&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Create a new topic in a forum supergroup.

- `icon_color`  -  RGB color int for the topic icon. Telegram supports six presets: <code>0x6FB9F0</code>, <code>0xFFD67E</code>, <code>0xCB86DB</code>, <code>0x8EEE98</code>, <code>0xFF93B2</code>, <code>0xFB6F5F</code>. Pass <code>None</code> to use the default blue.
- `icon_emoji_id`  -  Custom emoji document ID (from a premium sticker set) to use as the topic icon. Pass <code>None</code> to use a plain color circle.

```rust
// Simple topic
client.create_forum_topic(peer.clone(), "📢 Announcements", None, None).await?;

// Colored topic
client.create_forum_topic(
    peer.clone(),
    "🛠 Dev",
    Some(0x6FB9F0),
    None,
).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_forum_topic(peer: impl Into&lt;PeerRef&gt;, topic_id: i32, title: Option&lt;String&gt;, icon_emoji_id: Option&lt;i64&gt;, closed: Option&lt;bool&gt;, hidden: Option&lt;bool&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Update topic properties. Pass <code>None</code> for any field to leave it unchanged.

- `closed`  -  `Some(true)` to prevent new messages; `Some(false)` to reopen.
- `hidden`  -  Only valid for topic ID 1 (General). Hides it from the topic list.

```rust
// Rename a topic
client.edit_forum_topic(peer.clone(), 42, Some("New Name".into()), None, None, None).await?;

// Close a topic
client.edit_forum_topic(peer.clone(), 42, None, None, Some(true), None).await?;

// Hide the General topic
client.edit_forum_topic(peer.clone(), 1, None, None, None, Some(true)).await?;
```
</div>
</div>

---

## Delete topic history

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_forum_topic_history(peer: impl Into&lt;PeerRef&gt;, top_msg_id: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Delete all messages inside a forum topic. <code>top_msg_id</code> is the topic's root message ID (returned as <code>ForumTopic::Topic.id</code>). This is destructive and irreversible. The method automatically pages through the deletion until all messages are removed.

```rust
// Wipe the "Dev" topic (top_msg_id = 42)
client.delete_forum_topic_history(peer.clone(), 42).await?;
```
</div>
</div>

---

## Sending messages to a topic

To send a message inside a specific topic, set `reply_to` on the `InputMessage` to the topic's `top_msg_id`:

```rust
use ferogram::InputMessage;

// Send into topic #42
client.send_message(
    peer.clone(),
    InputMessage::text("Hello topic!").reply_to(Some(42)),
).await?;
```

---

## `ForumTopic` fields reference

```rust
if let tl::enums::ForumTopic::Topic(t) = topic {
    t.id            // i32: topic ID / top_msg_id
    t.title         // String: display name
    t.icon_color    // i32: RGB color
    t.icon_emoji_id // Option<i64>: custom emoji ID
    t.top_message   // i32: most recent message ID
    t.unread_count  // i32: unread messages
    t.unread_mentions_count // i32
    t.closed        // bool: no new messages allowed
    t.pinned        // bool: pinned in topic list
    t.short         // bool: General topic placeholder
}
```
