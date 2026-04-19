# Games & Payments

Bot API for HTML5 games (scores, high scores) and Telegram Payments (shipping and pre-checkout confirmations).

---

## Bots

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.start_bot(bot_user_id: i64, peer: impl Into&lt;PeerRef&gt;, start_param: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send a <code>/start</code> command to a bot with a deep-link parameter, as if
the user clicked a <code>t.me/BotUsername?start=param</code> link. The bot
receives the parameter in <code>Message.text</code> as <code>/start param</code>.

### Parameters

| Param | Type | Description |
|---|---|---|
| `bot_user_id` | `i64` | The user ID of the bot to start. Must match a bot account. |
| `peer` | `impl Into<PeerRef>` | The chat or user where the `/start` message is sent. Usually the same as the bot's own private chat (pass the bot's user ID). |
| `start_param` | `impl Into<String>` | The deep-link payload. Max 64 characters, only `A-Z a-z 0-9 _ -` are permitted by Telegram. |

### Examples

```rust
// Basic: open a bot in a private chat with a deep-link parameter
// bot_user_id is the numeric ID of the bot (not the username)
client.start_bot(bot_user_id, bot_user_id, "welcome").await?;
```

```rust
// Affiliate / referral tracking: encode a referrer ID in the param
let ref_param = format!("ref_{}", referrer_user_id);
client.start_bot(bot_user_id, bot_user_id, ref_param).await?;
```

```rust
// Launch a specific game flow
client.start_bot(bot_user_id, bot_user_id, "play").await?;
```

```rust
// Open the bot inside a group chat (bot must be a member)
client.start_bot(bot_user_id, group_peer, "group_hello").await?;
```

### Getting `bot_user_id`

`bot_user_id` is the numeric user ID of the bot, not its username string. You
can resolve it from the username once and cache it:

```rust
use ferogram::PeerRef;

let bot = client.resolve_username("MyBotUsername").await?;
let bot_user_id: i64 = match bot {
    PeerRef::UserId(id) => id,
    other => panic!("expected a user, got {:?}", other),
};
```

Or if you already have the bot's user object from an earlier API call:

```rust
let bot_user_id = user.id();
```

### What the bot receives

The bot's update handler sees a `Message` whose text is exactly
`/start <start_param>`. For example, if you pass `"ref_42"`:

```
/start ref_42
```

The bot can extract the param from `message.text.strip_prefix("/start ")`.

### Notes

- `start_param` is limited to 64 characters. Attempting a longer value will
  be rejected by the Telegram server with `START_PARAM_INVALID`.
- Allowed characters: `A-Z`, `a-z`, `0-9`, `_`, `-`. Spaces and special
  characters are not permitted.
- The call is idempotent from the server's perspective. Sending the same
  `/start param` twice triggers two separate updates on the bot side.
- For bots with `allow_zero_hash` disabled (the default), the bot's
  `access_hash` must already be in the peer cache. Call
  `client.resolve_username()` at least once before `start_bot` to populate
  the cache, or enable
  [`ExperimentalFeatures::allow_zero_hash`](../advanced/experimental-features.md).
</div>
</div>

---

## Games

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_game_score(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, user_id: i64, score: i32, force: bool, edit_message: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set a user's score in a game that was sent in a chat message.

| Param | Description |
|---|---|
| `peer` | Chat where the game message lives |
| `msg_id` | ID of the message containing the game |
| `user_id` | User whose score to update |
| `score` | New score value |
| `force` | If `true`, allow setting a score lower than the current one |
| `edit_message` | If `true`, the game message is edited to show the new score |

```rust
client.set_game_score(peer.clone(), msg_id, user_id, 42_000, false, true).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_game_high_scores(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, user_id: i64) → Result&lt;Vec&lt;tl::types::HighScore&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Retrieve the high-score table for a game message, anchored around <code>user_id</code>. Returns up to 5 entries centred on the specified user's position.

```rust
let scores = client.get_game_high_scores(peer.clone(), msg_id, user_id).await?;
for s in &scores {
    println!("#{}  -  user {}  -  {} pts", s.pos, s.user_id, s.score);
}
```

### `HighScore` fields

| Field | Type | Description |
|---|---|---|
| `pos` | `i32` | Rank position (1-based) |
| `user_id` | `i64` | User ID |
| `score` | `i32` | Score value |
</div>
</div>

---

## Payments

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.answer_shipping_query(query_id: i64, error: Option&lt;String&gt;, shipping_options: Option&lt;Vec&lt;tl::enums::ShippingOption&gt;&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Respond to a <code>ShippingQuery</code> update that a bot receives when a user provides their shipping address for an invoice with <code>is_flexible = true</code>.

- Pass `error = None` and `shipping_options = Some(...)` to confirm available shipping options.
- Pass `error = Some("message")` and `shipping_options = None` to reject the query with an error shown to the user.

```rust
// Accept with two shipping options
client.answer_shipping_query(
    query.query_id,
    None,
    Some(vec![
        tl::enums::ShippingOption::ShippingOption(tl::types::ShippingOption {
            id: "standard".into(),
            title: "Standard (5-7 days)".into(),
            prices: vec![
                tl::enums::LabeledPrice::LabeledPrice(tl::types::LabeledPrice {
                    label: "Shipping".into(),
                    amount: 500,  // in smallest currency unit
                }),
            ],
        }),
    ]),
).await?;

// Reject (address not serviceable)
client.answer_shipping_query(
    query.query_id,
    Some("We don't ship to this address.".into()),
    None,
).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.answer_precheckout_query(query_id: i64, ok: bool, error_message: Option&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Confirm or reject a pre-checkout query. You receive this just before Telegram finalises a payment  -  use it to verify stock availability, validate the order, etc.

- `ok: true`  -  approve the payment; Telegram completes it.
- `ok: false`  -  reject; pass `error_message` to explain why.

You **must** answer within 10 seconds or the payment times out.

```rust
// Approve
client.answer_precheckout_query(query.query_id, true, None).await?;

// Reject
client.answer_precheckout_query(
    query.query_id,
    false,
    Some("Item is out of stock.".into()),
).await?;
```
</div>
</div>

---

## Update variants

Payments generate update variants that you handle in your update loop:

```rust
Update::ShippingQuery(q) => {
    // q.query_id, q.user_id, q.payload, q.shipping_address
    client.answer_shipping_query(q.query_id, None, Some(options)).await?;
}

Update::PreCheckoutQuery(q) => {
    // q.query_id, q.user_id, q.currency, q.total_amount, q.payload
    client.answer_precheckout_query(q.query_id, true, None).await?;
}
```
