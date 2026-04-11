# Search

`ferogram` provides two fluent search builders:

- **`SearchBuilder`**: search within a single peer (`client.search()`)
- **`GlobalSearchBuilder`**: search across all dialogs (`client.search_global_builder()`)

Both builders return `Vec<IncomingMessage>` from `.fetch(&client).await?`.

---

## `SearchBuilder`: in-chat search

```rust
use ferogram_tl_types::enums::MessagesFilter;

let results = client
    .search(peer.clone(), "rust async")  // peer: impl Into<PeerRef>
    .limit(50)
    .fetch(&client)
    .await?;

for msg in &results {
    println!("[{}] {}", msg.id, msg.message);
}
```

`client.search(peer, query)` accepts any `impl Into<PeerRef>`: a `&str` username, a `tl::enums::Peer`, or a numeric `i64` ID.

### All `SearchBuilder` methods

| Method | Default | Description |
|---|---|---|
| `.limit(n: i32)` | `100` | Maximum results to return |
| `.min_date(ts: i32)` | `0` | Only messages at or after this Unix timestamp |
| `.max_date(ts: i32)` | `0` | Only messages at or before this Unix timestamp |
| `.offset_id(id: i32)` | `0` | Start from this message ID (pagination) |
| `.add_offset(n: i32)` | `0` | Additional offset for fine pagination |
| `.max_id(id: i32)` | `0` | Only messages with ID ≤ `max_id` |
| `.min_id(id: i32)` | `0` | Only messages with ID ≥ `min_id` |
| `.filter(f: MessagesFilter)` | `Empty` | Filter by media type |
| `.sent_by_self()` |: | Only messages sent by the logged-in user |
| `.from_peer(peer: InputPeer)` | `None` | Only messages from this specific sender |
| `.top_msg_id(id: i32)` | `None` | Restrict search to a forum topic thread |
| `.fetch(&client)` |: | Execute: returns `Vec<IncomingMessage>` |

### Filter by media type

```rust
use ferogram_tl_types::enums::MessagesFilter;

// Photos only
let photos = client
    .search(peer.clone(), "")
    .filter(MessagesFilter::InputMessagesFilterPhotos)
    .limit(30)
    .fetch(&client)
    .await?;

// Documents only
let docs = client
    .search(peer.clone(), "report")
    .filter(MessagesFilter::InputMessagesFilterDocument)
    .fetch(&client)
    .await?;

// Voice messages
let voices = client
    .search(peer.clone(), "")
    .filter(MessagesFilter::InputMessagesFilterVoice)
    .fetch(&client)
    .await?;
```

### Common `MessagesFilter` values

| Filter | Matches |
|---|---|
| `InputMessagesFilterEmpty` | All messages (default) |
| `InputMessagesFilterPhotos` | Photos |
| `InputMessagesFilterVideo` | Videos |
| `InputMessagesFilterDocument` | Documents / files |
| `InputMessagesFilterAudio` | Audio files |
| `InputMessagesFilterVoice` | Voice messages |
| `InputMessagesFilterRoundVideo` | Video notes (round videos) |
| `InputMessagesFilterUrl` | Messages with URLs |
| `InputMessagesFilterMyMentions` | Messages where you were @mentioned |
| `InputMessagesFilterPinned` | Pinned messages |
| `InputMessagesFilterGeo` | Messages with location |

### Date-range search

```rust
// Messages from the last 7 days
let week_ago = (std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_secs() - 7 * 86400) as i32;

let results = client
    .search(peer.clone(), "error")
    .min_date(week_ago)
    .limit(100)
    .fetch(&client)
    .await?;
```

### Search from a specific sender

```rust
// Messages sent by yourself
let mine = client
    .search(peer.clone(), "")
    .sent_by_self()
    .fetch(&client)
    .await?;

// Messages from a specific InputPeer
let alice_peer = tl::enums::InputPeer::User(tl::types::InputPeerUser {
    user_id: alice_id,
    access_hash: alice_hash,
});

let from_alice = client
    .search(peer.clone(), "hello")
    .from_peer(alice_peer)
    .fetch(&client)
    .await?;
```

### Forum topic search

```rust
let results = client
    .search(supergroup_peer.clone(), "query")
    .top_msg_id(topic_msg_id)
    .fetch(&client)
    .await?;
```

### Pagination

```rust
let mut offset_id = 0;

loop {
    let page = client
        .search(peer.clone(), "keyword")
        .offset_id(offset_id)
        .limit(50)
        .fetch(&client)
        .await?;

    if page.is_empty() { break; }

    for msg in &page {
        println!("[{}] {}", msg.id, msg.message);
    }

    // Move the cursor to the oldest message in this page
    offset_id = page.iter().map(|m| m.id).min().unwrap_or(0);
}
```

---

## `GlobalSearchBuilder`: search all chats

```rust
let results = client
    .search_global_builder("rust async")
    .limit(30)
    .fetch(&client)
    .await?;

for msg in &results {
    println!("[{:?}] [{}] {}", msg.peer_id, msg.id, msg.message);
}
```

### All `GlobalSearchBuilder` methods

| Method | Default | Description |
|---|---|---|
| `.limit(n: i32)` | `100` | Maximum results |
| `.min_date(ts: i32)` | `0` | Only messages at or after this timestamp |
| `.max_date(ts: i32)` | `0` | Only messages at or before this timestamp |
| `.offset_rate(r: i32)` | `0` | Pagination: rate from last response |
| `.offset_id(id: i32)` | `0` | Pagination: message ID from last response |
| `.folder_id(id: i32)` | `None` | Restrict to a specific dialog folder |
| `.broadcasts_only(v: bool)` | `false` | Only search channels |
| `.groups_only(v: bool)` | `false` | Only search groups / supergroups |
| `.users_only(v: bool)` | `false` | Only search private chats / bots |
| `.filter(f: MessagesFilter)` | `Empty` | Filter by media type |
| `.fetch(&client)` |: | Execute: returns `Vec<IncomingMessage>` |

### Filter by chat type

```rust
// Channels only
let channel_results = client
    .search_global_builder("announcement")
    .broadcasts_only(true)
    .limit(20)
    .fetch(&client)
    .await?;

// Groups / supergroups only
let group_results = client
    .search_global_builder("discussion")
    .groups_only(true)
    .fetch(&client)
    .await?;

// Private chats and bots only
let dm_results = client
    .search_global_builder("invoice")
    .users_only(true)
    .fetch(&client)
    .await?;
```

### Combined filters

```rust
// Photo messages from channels, last 30 days
let cutoff = (chrono::Utc::now().timestamp() - 30 * 86400) as i32;

let photos = client
    .search_global_builder("")
    .broadcasts_only(true)
    .filter(MessagesFilter::InputMessagesFilterPhotos)
    .min_date(cutoff)
    .limit(50)
    .fetch(&client)
    .await?;
```

---

## Simple one-liner methods (no builder)

For quick lookups that don't need date/filter options:

```rust
// Per-chat search: returns Vec<IncomingMessage>
let results = client.search_messages(peer.clone(), "query", 20).await?;

// Global search: returns Vec<IncomingMessage>
let results = client.search_global("ferogram rust", 10).await?;
```
