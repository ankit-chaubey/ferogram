# Inline Mode

Inline mode lets users type `@yourbot query` in any chat and receive results. ferogram supports both sides: **receiving queries** (bot) and **sending queries** (user account).

---

## Receiving inline queries (bot side)

### Via update stream

```rust
Update::InlineQuery(iq) => {
    let query    = iq.query();    // &str: what the user typed
    let query_id = iq.query_id;  // i64: must be passed to answer_inline_query

    let results = vec![
        tl::enums::InputBotInlineResult::InputBotInlineResult(
            tl::types::InputBotInlineResult {
                id:    "1".into(),
                r#type: "article".into(),
                title: Some("Result title".into()),
                description: Some(query.to_string()),
                url: None, thumb: None, content: None,
                send_message: tl::enums::InputBotInlineMessage::Text(
                    tl::types::InputBotInlineMessageText {
                        no_webpage: false, invert_media: false,
                        message: query.to_string(),
                        entities: None, reply_markup: None,
                    },
                ),
            },
        ),
    ];

    // cache_time: seconds, is_personal: false, next_offset: None
    client.answer_inline_query(query_id, results, 30, false, None).await?;
}
```

### Via `InlineQueryIter`

For a more structured approach, use the dedicated iterator:

```rust
use ferogram::inline_iter::InlineQueryIter;

let mut iter = client.iter_inline_queries();
while let Some(iq) = iter.next().await {
    println!("Query: {}", iq.query());
    // answer it...
}
```

`InlineQueryIter` is backed by the update stream: it filters and yields only `InlineQuery` updates.

---

## `InlineQuery` fields

```rust
iq.query()       // &str: the search text
iq.query_id      // i64
iq.user_id       // i64: who sent the query
iq.offset        // String: pagination offset
iq.peer          // Option<tl::enums::Peer>: chat context the user is in (None if not shared)
```

`peer` is populated when the bot has access to the chat  -  use it to tailor results per chat type.

---

## Receiving inline sends (bot side)

When a user selects a result from your bot's inline mode, you get `Update::InlineSend`:

```rust
Update::InlineSend(is) => {
    println!("User {} chose result '{}'", is.user_id, is.id);
    println!("Original query: {}", is.query);
    // is.msg_id is Some(...) when the message is still editable
}
```

### `InlineSend` fields

| Field | Type | Description |
|---|---|---|
| `is.user_id` | `i64` | Who picked the inline result |
| `is.query` | `String` | The original query text |
| `is.id` | `String` | The result ID that was chosen |
| `is.msg_id` | `Option<tl::enums::InputBotInlineMessageId>` | Present when the sent message can still be edited |

### Editing the sent inline message

```rust
Update::InlineSend(is) => {
    is.edit_message(&client, updated_msg).await?;
}
```

---

## Sending inline queries (user account side)

A **user account** can invoke another bot's inline mode with `client.inline_query()` and iterate the results:

```rust
use ferogram::inline_iter::InlineResultIter;

let mut iter = client
    .inline_query("@gif", "cute cats")
    .peer(input_peer_for_target_chat)
    .await?;

while let Some(result) = iter.next().await? {
    println!("Result: {:?}: {:?}", result.id(), result.title());

    // Send the first result to a chat
    result.send(target_peer.clone()).await?;
    break;
}
```

### `InlineResult` methods

| Method | Return | Description |
|---|---|---|
| `result.id()` | `&str` | Result ID string |
| `result.title()` | `Option<&str>` | Display title |
| `result.description()` | `Option<&str>` | Display description |
| `result.raw` | `tl::enums::BotInlineResult` | Raw TL object |
| `result.send(peer)` | `async → ()` | Send this result to a chat |

### `InlineResultIter` methods

| Method | Description |
|---|---|
| `client.inline_query(bot, query)` | Create builder, returns `InlineResultIter` |
| `iter.peer(input_peer)` | Set the chat context (required by some bots) |
| `iter.next()` | `async → Option<InlineResult>`: fetch next result |

---

## `answer_inline_query` parameters

```rust
client.answer_inline_query(
    query_id,   // i64: from InlineQuery
    results,    // Vec<InputBotInlineResult>
    30,         // cache_time: i32: seconds to cache results
    false,      // is_personal: bool: different results per user?
    None,       // next_offset: Option<String>: for pagination
).await?;
```
