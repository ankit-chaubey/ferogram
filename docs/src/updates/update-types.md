# Update Types

`stream.next().await` yields `Option<Update>`. `Update` is `#[non_exhaustive]`: always include `_ => {}`.

```rust
use ferogram::update::Update;

while let Some(update) = stream.next().await {
    match update {
        // Messages
        Update::NewMessage(msg)       => { /* IncomingMessage */ }
        Update::MessageEdited(msg)    => { /* IncomingMessage */ }
        Update::MessageDeleted(del)   => { /* MessageDeleted */ }

        // Bot interactions
        Update::CallbackQuery(cb)     => { /* CallbackQuery */ }
        Update::InlineQuery(iq)       => { /* InlineQuery */ }
        Update::InlineSend(is)        => { /* InlineSend */ }

        // Presence
        Update::UserTyping(action)    => { /* UserTyping */ }
        Update::UserStatus(status)    => { /* UserStatus */ }

        // Group/channel events
        Update::ParticipantUpdate(p)  => { /* ParticipantUpdate */ }
        Update::JoinRequest(jr)       => { /* JoinRequest */ }
        Update::MessageReaction(mr)   => { /* MessageReaction */ }
        Update::PollVote(pv)          => { /* PollVote */ }
        Update::BotStopped(bs)        => { /* BotStopped */ }

        // Payments
        Update::ShippingQuery(sq)     => { /* ShippingQuery */ }
        Update::PreCheckoutQuery(pcq) => { /* PreCheckoutQuery */ }

        // Boosts
        Update::ChatBoost(cb)         => { /* ChatBoost */ }

        // Raw passthrough
        Update::Raw(raw)              => { /* RawUpdate */ }

        _ => {}  // required: Update is #[non_exhaustive]
    }
}
```

---

## `MessageDeleted`

```rust
Update::MessageDeleted(del) => {
    let ids: Vec<i32> = del.into_messages();
}
```

| Method | Return | Description |
|---|---|---|
| `del.into_messages()` | `Vec<i32>` | IDs of deleted messages |

---

## `CallbackQuery`

See the full [Callback Queries](./callbacks.md) page.

```rust
cb.query_id      // i64
cb.user_id       // i64
cb.msg_id        // Option<i32>
cb.data()        // Option<&str>
cb.answer()      // → Answer builder
cb.answer_flat(&client, text)
cb.answer_alert(&client, text)
```

---

## `InlineQuery`

```rust
iq.query_id      // i64
iq.user_id       // i64
iq.query()       // &str: the typed query
iq.offset        // String: pagination offset
```

Answer with `client.answer_inline_query(...)`. See [Inline Mode](./inline-mode.md).

---

## `InlineSend`

Fires when a user picks a result from your bot's inline mode.

```rust
is.result_id     // String: which result was chosen
is.user_id       // i64
is.query         // String: original query

// Edit the message the inline result was sent as
is.edit_message(&client, updated_input_msg).await?;
```

---

## `UserTyping`

```rust
Update::UserTyping(action) => {
    action.peer      // tl::enums::Peer: the chat
    action.user_id   // Option<i64>
    action.action    // tl::enums::SendMessageAction
}
```

---

## `UserStatus`

```rust
Update::UserStatus(status) => {
    status.user_id  // i64
    status.status   // tl::enums::UserStatus
    // variants: UserStatusOnline, UserStatusOffline, UserStatusRecently, etc.
}
```

---

## `ParticipantUpdate`

Fires when a user joins, leaves, or has their rights changed in a group or channel.

```rust
Update::ParticipantUpdate(p) => {
    p.peer      // tl::enums::Peer: the chat
    p.user_id   // i64
    // p.prev_participant / p.new_participant: Option<tl::enums::ChannelParticipant>
}
```

---

## `JoinRequest`

Fires when a user submits a join request to a group or channel.

```rust
Update::JoinRequest(jr) => {
    jr.peer     // tl::enums::Peer
    jr.user_id  // i64
    // Approve/decline via client.join_request(peer, user_id, approve)
}
```

---

## `MessageReaction`

Fires when a reaction is added or removed on a message.

```rust
Update::MessageReaction(mr) => {
    mr.peer    // tl::enums::Peer
    mr.msg_id  // i32
    // mr.reactions: list of current reactions
}
```

---

## `PollVote`

Fires when a user votes in a poll.

```rust
Update::PollVote(pv) => {
    pv.poll_id  // i64
    pv.user_id  // i64
    // pv.options: Vec<Vec<u8>> - option IDs chosen
}
```

---

## `BotStopped`

Fires when a user blocks or unblocks the bot.

```rust
Update::BotStopped(bs) => {
    bs.user_id  // i64
    bs.stopped  // bool: true = blocked, false = unblocked
}
```

---

## `ShippingQuery`

Fires during the payment flow when the user provides a shipping address (for physical goods).

```rust
Update::ShippingQuery(sq) => {
    // sq.query_id, sq.user_id, sq.payload, sq.shipping_address
    client.answer_shipping_query(sq.query_id, true, shipping_options, None).await?;
}
```

---

## `PreCheckoutQuery`

Fires just before a payment is confirmed.

```rust
Update::PreCheckoutQuery(pcq) => {
    // pcq.query_id, pcq.user_id, pcq.currency, pcq.total_amount, pcq.payload
    client.answer_precheckout_query(pcq.query_id, true, None).await?;
}
```

---

## `ChatBoost`

Fires when a channel receives a boost event.

```rust
Update::ChatBoost(cb) => {
    cb.peer   // tl::enums::Peer: the channel
    // cb.boost: boost details
}
```

---

## `RawUpdate`

Any TL update that doesn't map to a typed variant:

```rust
Update::Raw(raw) => {
    raw.update   // tl::enums::Update: the raw TL object
}
```

---

## Raw update stream

If you need all updates unfiltered:

```rust
let mut stream = client.stream_updates();
while let Some(raw) = stream.next_raw().await {
    println!("{:?}", raw.update);
}
```
