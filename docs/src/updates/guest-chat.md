# Guest Chat Queries

`Update::GuestChatQuery` fires when a user invites the bot into a guest-chat context (`updateBotGuestChatQuery`). Bots only.

---

## Receiving the update

```rust
use ferogram::update::Update;

while let Some(update) = stream.next().await {
    if let Update::GuestChatQuery(q) = update {
        println!("query_id: {}", q.query_id);
        println!("from: {:?}", q.message.sender_id());
        println!("text: {}", q.message.text());
    }
}
```

`GuestChatQuery` derefs to `IncomingMessage`, so all the usual accessors (`text()`, `sender_id()`, `chat_id()`, `date()`, etc.) work directly on `q`.

---

## Fields

| Field | Type | Description |
|---|---|---|
| `query_id` | `i64` | ID used to submit the answer |
| `message` | `IncomingMessage` | The message that triggered the query |
| `reference_messages` | `Vec<IncomingMessage>` | Prior context messages provided by Telegram, if any |
| `qts` | `i32` | QTS sequence number for this update |

---

## Answering

Call `q.answer()` to get a `GuestChatAnswer` builder, then call `.send(&client)`.

```rust
if let Update::GuestChatQuery(q) = update {
    q.answer()
        .article("My result")
        .text("Answer body")
        .send(&client)
        .await?;
}
```

### Result kinds

| Method | Description |
|---|---|
| `.article(title)` | Text article result |
| `.photo(input_photo)` | Existing photo |
| `.document(input_doc, title)` | Existing document |
| `.game(short_name)` | Game result |
| `.location(lat, long)` | Geographic location |
| `.venue(lat, long, title, address, ...)` | Venue |
| `.contact(phone, first_name, last_name)` | Contact card |
| `.webpage(url)` | Web page preview |
| `.invoice(title, description, ...)` | Invoice |
| `.raw(InputBotInlineResult)` | Fully constructed TL result |

### Message content

| Method | Description |
|---|---|
| `.text(str)` | Text content for the result message |
| `.caption(str)` | Caption for media results |
| `.no_webpage(bool)` | Disable link preview |
| `.invert_media(bool)` | Invert media position |
| `.entities(vec)` | Manual `MessageEntity` list |
| `.reply_markup(markup)` | Attach inline keyboard |

### Other builder methods

| Method | Description |
|---|---|
| `.id(str)` | Custom result ID (auto-generated if omitted) |
| `.description(str)` | Short description shown under title |
| `.url(str)` | URL for the result |
| `.thumb(InputWebDocument)` | Thumbnail |

---

## Checking if a bot supports guest chat

In the `User` type, `bot_guestchat` (`flags2.19`) is set when the bot has declared guest-chat support. This is a Layer 225 addition.
