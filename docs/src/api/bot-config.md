# Bot Configuration & Special Features

This page covers bot-specific configuration: setting the command menu, editing bot profile info, QR-code login, sending dice, invoices, and starting bots programmatically.

---

## Command menu

The command menu appears in the Telegram UI when users tap the `/` button or the bot's profile.

### Set commands

```rust
client.set_bot_commands(
    &[
        ("start", "Start the bot"),
        ("help",  "Show help"),
        ("order", "Place an order"),
    ],
    None,   // scope: None = default (all users, all chats)
    "",     // lang_code: "" = default language
).await?;
```

**Scopes** limit where the commands appear. Pass any `tl::enums::BotCommandScope` variant:

```rust
use ferogram_tl_types as tl;

// Only show /admin_ban in group chats
client.set_bot_commands(
    &[("admin_ban", "Ban a user")],
    Some(tl::enums::BotCommandScope::Chats),
    "",
).await?;
```

### Delete commands

```rust
// Remove the default command list
client.delete_bot_commands(None, "").await?;
```

---

## Bot profile info

Set the bot's display name, about text, and description for a given language:

```rust
client.set_bot_info(
    Some("My Awesome Bot"),         // name shown in chat header
    Some("I help with orders"),     // about text (bio)
    Some("Send /start to begin."),  // description shown before first message
    "",                             // lang_code: "" = default locale
).await?;
```

Pass `None` for any field you do not want to change.

Retrieve the current values:

```rust
let info = client.get_bot_info("").await?;
println!("Name: {}", info.name);
println!("About: {}", info.about);
println!("Description: {}", info.description);
```

---

## Start a bot programmatically

Send `/start start_param` as if a user pressed a deep-link button:

```rust
// bot_user_id: the bot's user ID
// peer: the chat where /start is sent
// start_param: the payload after the link (empty string for plain /start)
client.start_bot(
    bot_user_id,
    "@somebot",
    "ref_12345",
).await?;
```

This is equivalent to the user clicking `https://t.me/somebot?start=ref_12345`.

---

## Send a dice / animated emoji

```rust
// Classic 🎲 dice (value 1-6)
client.send_dice("@mychat", "🎲").await?;

// Dart 🎯 (value 1-6)
client.send_dice("@mychat", "🎯").await?;

// Basketball 🏀 (value 1-5)
client.send_dice("@mychat", "🏀").await?;

// Slot machine 🎰 (value 1-64)
client.send_dice("@mychat", "🎰").await?;
```

Supported emoticons: `🎲`, `🎯`, `🏀`, `⚽`, `🎳`, `🎰`.

---

## QR-code login (user accounts)

Generate a login token encoded as a QR code, then poll until the user scans it.

```rust
// 1. Generate a token
let (token_bytes, expires_ts) = client.export_login_token().await?;

// 2. Encode as tg://login?token=<base64url> and display as QR code
let b64 = base64::encode_config(&token_bytes, base64::URL_SAFE_NO_PAD);
let url = format!("tg://login?token={}", b64);
println!("Scan: {url}");

// 3. Poll until the user scans it
loop {
    match client.check_qr_login(token_bytes.clone()).await? {
        Some(username) => {
            println!("Logged in as: {username}");
            client.save_session().await?;
            break;
        }
        None => {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
}
```

`export_login_token` handles DC migration automatically. Returns `(vec![], 0)` if the user already scanned before you called it.

---

## Send an invoice (payments)

Send a payment invoice to a chat (bots only):

```rust
client.send_invoice(
    "@user",
    "Premium subscription",          // title
    "One month of premium access",   // description
    "sub_monthly",                   // payload (your internal ID)
    "USD",                           // currency
    &[
        ("1 Month Premium", 999),    // (label, amount in cents)
    ],
    None,                            // photo_url
    false,                           // need_name
    false,                           // need_phone
    false,                           // need_email
    false,                           // need_shipping_address
    false,                           // is_flexible (shipping address changes price)
).await?;
```

Handle the resulting shipping and pre-checkout queries via `Update::ShippingQuery` and `Update::PreCheckoutQuery`:

```rust
Update::ShippingQuery(sq) => {
    // Approve with shipping options, or decline with an error
    client.answer_shipping_query(
        sq.query_id,
        None,  // no error
        Some(vec![/* ShippingOption... */]),
    ).await?;
}

Update::PreCheckoutQuery(pcq) => {
    // Confirm the payment
    client.answer_precheckout_query(pcq.query_id, true, None).await?;
    // Or decline:
    // client.answer_precheckout_query(pcq.query_id, false, Some("Out of stock".into())).await?;
}
```

See the [Telegram Payments documentation](https://core.telegram.org/bots/payments) for the full payment flow.
