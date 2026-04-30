# Inline Keyboards & Reply Markup

`ferogram` ships with two high-level keyboard builders: `InlineKeyboard` and `ReplyKeyboard`: so you never have to construct raw TL types by hand.

Both builders are in `ferogram::keyboard` and re-exported at the crate root:

```rust
use ferogram::keyboard::{Button, InlineKeyboard, ReplyKeyboard};
```

---

## `InlineKeyboard`: buttons attached to a message

Inline keyboards appear below a message and trigger `Update::CallbackQuery` when tapped.

```rust
use ferogram::keyboard::{Button, InlineKeyboard};
use ferogram::InputMessage;

let kb = InlineKeyboard::new()
    .row([
        Button::callback("✅ Yes", b"confirm:yes"),
        Button::callback("❌ No",  b"confirm:no"),
    ])
    .row([
        Button::url("📖 Docs", "https://docs.rs/ferogram"),
    ]);

client.send_message(peer.clone(), InputMessage::text("Do you want to proceed?").keyboard(kb)).await?;
```

### `InlineKeyboard` methods

| Method | Description |
|---|---|
| `InlineKeyboard::new()` | Create an empty keyboard |
| `.row(buttons)` | Append a row; accepts any `IntoIterator<Item = Button>` |
| `.into_markup()` | Convert to `tl::enums::ReplyMarkup` |

`InlineKeyboard` implements `Into<tl::enums::ReplyMarkup>`, so you can also pass it directly to `InputMessage::reply_markup()`.

---

## `Button`: all button types

### Callback, URL, and common types

```rust
// Sends data to your bot as Update::CallbackQuery (max 64 bytes)
Button::callback("✅ Confirm", b"action:confirm")

// Opens URL in a browser
Button::url("🌐 Website", "https://example.com")

// Copy text to clipboard (Telegram 10.3+)
Button::copy_text("📋 Copy code", "PROMO2024")

// Login-widget: authenticates user before opening URL
Button::url_auth("🔐 Login", "https://example.com/auth", None, bot_input_user)

// Opens bot inline mode in the current chat with query pre-filled
Button::switch_inline("🔍 Search here", "default query")

// Chat picker so user can choose which chat to use inline mode in
Button::switch_elsewhere("📤 Share", "")

// Telegram Mini App with full JS bridge
Button::webview("🚀 Open App", "https://myapp.example.com")

// Simple webview without JS bridge
Button::simple_webview("ℹ️ Info", "https://info.example.com")

// Plain text for reply keyboards
Button::text("📸 Send photo")

// Launch a Telegram game (bots only)
Button::game("🎮 Play")

// Payment buy button (bots only, used with invoice)
Button::buy("💳 Pay $4.99")
```

### Reply-keyboard-only buttons

```rust
// Shares user's phone number on tap
Button::request_phone("📞 Share my number")

// Shares user's location on tap
Button::request_geo("📍 Share location")

// Opens poll creation interface
Button::request_poll("📊 Create poll")

// Forces quiz mode in poll creator
Button::request_quiz("🧠 Create quiz")
```

### Escape hatch

```rust
// Get the underlying tl::enums::KeyboardButton
let raw = Button::callback("x", b"x").into_raw();
```

---

## `ReplyKeyboard`: replacement keyboard

A reply keyboard replaces the user's text input keyboard until dismissed.
The user's tap arrives as a plain-text `Update::NewMessage`.

```rust
use ferogram::keyboard::{Button, ReplyKeyboard};

let kb = ReplyKeyboard::new()
    .row([
        Button::text("📸 Photo"),
        Button::text("📄 Document"),
    ])
    .row([Button::text("❌ Cancel")])
    .resize()      // shrink to fit content (recommended)
    .single_use(); // hide after one press

client.send_message(peer.clone(), InputMessage::text("Choose file type:").keyboard(kb))
    .await?;
```

### `ReplyKeyboard` methods

| Method | Description |
|---|---|
| `ReplyKeyboard::new()` | Create an empty keyboard |
| `.row(buttons)` | Append a row of buttons |
| `.resize()` | Shrink keyboard to fit button count |
| `.single_use()` | Dismiss after one tap |
| `.selective()` | Show only to mentioned/replied users |
| `.into_markup()` | Convert to `tl::enums::ReplyMarkup` |

---

## Remove keyboard

```rust
use ferogram_tl_types as tl;

let remove = tl::enums::ReplyMarkup::ReplyKeyboardHide(
    tl::types::ReplyKeyboardHide { selective: false }
);
client
    .send_message(peer.clone(), InputMessage::text("Done.").reply_markup(remove))
    .await?;
```

---

## Answer callback queries

Always answer every `CallbackQuery`: Telegram shows a loading spinner until you do.

```rust
Update::CallbackQuery(cb) => {
    let data = cb.data().unwrap_or(b"");
    match data {
        b"confirm:yes" => client.answer_callback_query(cb.query_id, Some("✅ Done!"), false).await?,
        b"confirm:no"  => client.answer_callback_query(cb.query_id, Some("❌ Cancelled"), false).await?,
        _              => client.answer_callback_query(cb.query_id, None, false).await?,
    }
}
```

Pass `alert: true` to show a popup alert instead of a toast:

```rust
client.answer_callback_query(cb.query_id, Some("⛔ Access denied"), true).await?;
```

---

## Legacy raw TL pattern (still works)

If you prefer constructing TL types directly:

```rust
fn inline_kb(rows: Vec<Vec<tl::enums::KeyboardButton>>) -> tl::enums::ReplyMarkup {

```rust
use ferogram_tl_types as tl;

fn inline_kb(rows: Vec<Vec<tl::enums::KeyboardButton>>) -> tl::enums::ReplyMarkup {
    tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup {
        rows: rows.into_iter().map(|buttons|
            tl::enums::KeyboardButtonRow::KeyboardButtonRow(
                tl::types::KeyboardButtonRow { buttons }
            )
        ).collect(),
    })
}

fn btn_cb(text: &str, data: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Callback(tl::types::KeyboardButtonCallback {
        requires_password: false,
        style:             None,
        text:              text.into(),
        data:              data.as_bytes().to_vec(),
    })
}

fn btn_url(text: &str, url: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Url(tl::types::KeyboardButtonUrl {
        style: None,
        text:  text.into(),
        url:   url.into(),
    })
}
```

## Send with keyboard

```rust
let kb = inline_kb(vec![
    vec![btn_cb("✅ Yes", "confirm:yes"), btn_cb("❌ No", "confirm:no")],
    vec![btn_url("🌐 Docs", "https://github.com/ankit-chaubey/ferogram")],
]);

let (text, entities) = parse_markdown("**Do you want to proceed?**");
let msg = InputMessage::text(text)
    .entities(entities)
    .reply_markup(kb);

client.send_message(peer, msg).await?;
```

## All button types

| Type | Constructor | Description |
|---|---|---|
| Callback | `KeyboardButtonCallback` | Triggers `CallbackQuery` with custom data |
| URL | `KeyboardButtonUrl` | Opens a URL in the browser |
| Web App | `KeyboardButtonSimpleWebView` | Opens a Telegram Web App |
| Switch Inline | `KeyboardButtonSwitchInline` | Opens inline mode with a query |
| Request Phone | `KeyboardButtonRequestPhone` | Requests the user's phone number |
| Request Location | `KeyboardButtonRequestGeoLocation` | Requests location |
| Request Poll | `KeyboardButtonRequestPoll` | Opens poll creator |
| Request Peer | `KeyboardButtonRequestPeer` | Requests peer selection |
| Game | `KeyboardButtonGame` | Opens a Telegram game |
| Buy | `KeyboardButtonBuy` | Purchase button for payments |
| Copy | `KeyboardButtonCopy` | Copies text to clipboard |

### Switch Inline button

Opens the bot's inline mode in the current or another chat:

```rust
tl::enums::KeyboardButton::SwitchInline(tl::types::KeyboardButtonSwitchInline {
    same_peer:  false, // false = let user pick any chat
    text:       "🔍 Search with me".into(),
    query:      "default query".into(),
    peer_types: None,
})
```

### Web App button

```rust
tl::enums::KeyboardButton::SimpleWebView(tl::types::KeyboardButtonSimpleWebView {
    text: "Open App".into(),
    url:  "https://myapp.example.com".into(),
})
```

## Reply keyboard (replaces user's keyboard)

```rust
let reply_kb = tl::enums::ReplyMarkup::ReplyKeyboardMarkup(
    tl::types::ReplyKeyboardMarkup {
        resize:      true,       // shrink to fit buttons
        single_use:  true,       // hide after one tap
        selective:   false,      // show to everyone
        persistent:  false,      // don't keep after message
        placeholder: Some("Choose an option…".into()),
        rows: vec![
            tl::enums::KeyboardButtonRow::KeyboardButtonRow(
                tl::types::KeyboardButtonRow {
                    buttons: vec![
                        tl::enums::KeyboardButton::KeyboardButton(
                            tl::types::KeyboardButton { text: "🍕 Pizza".into() }
                        ),
                        tl::enums::KeyboardButton::KeyboardButton(
                            tl::types::KeyboardButton { text: "🍔 Burger".into() }
                        ),
                    ]
                }
            ),
            tl::enums::KeyboardButtonRow::KeyboardButtonRow(
                tl::types::KeyboardButtonRow {
                    buttons: vec![
                        tl::enums::KeyboardButton::KeyboardButton(
                            tl::types::KeyboardButton { text: "❌ Cancel".into() }
                        ),
                    ]
                }
            ),
        ],
    }
);
```

The user's choices arrive as plain text `NewMessage` updates.

## Remove keyboard

```rust
let remove = tl::enums::ReplyMarkup::ReplyKeyboardHide(
    tl::types::ReplyKeyboardHide { selective: false }
);
let msg = InputMessage::text("Keyboard removed.").reply_markup(remove);
```

## Button data format

Telegram limits callback button data to **64 bytes**. Use compact, parseable formats:

```rust
// Good: structured, compact
"vote:yes"
"page:3"
"item:42:delete"
"menu:settings:notifications"

// Bad: verbose
"user_clicked_the_settings_button"
```
