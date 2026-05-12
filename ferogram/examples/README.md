# Examples

Hey, welcome! If this is your first time here, start from the top. The examples are roughly ordered from simplest to most involved, so you can follow along naturally.

Before anything runs you need two things: an `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). Log in there, go to API development tools, and create an app. Copy those two values into the constants at the top of whichever example you want to run. That's a one-time thing.

After that, most examples ask for your phone number on the first run, send you a login code, and save a session file so you don't have to do it again.

---

## Want a full userbot out of the box?

Check out **[FeroUB](https://github.com/ankit-chaubey/feroub)** - a small, fast Telegram userbot built on ferogram. If you just want something that works without building it yourself, that's the one to grab. The examples below are more about understanding how ferogram works piece by piece.

---

## The basics (start here)

### `hello_self.rs`

The simplest thing ferogram can do. Logs in and sends "Hello from ferogram!" to your Saved Messages. That's it. Uses `quick_connect` so there's basically nothing to configure.

```
cargo run --example hello_self
```

Good first run. Once this works, try [`chat_history`](#chat_historyrs) to read messages back, or [`serverless_userbot`](#serverless_userbotrs) if you want to try a string session.

### `echo_bot.rs`

A bot that echoes every text message sent to it back to the sender. Runs as a bot (not your personal account), so it only sees messages people send directly to it. Safe to leave running.

```
cargo run --example echo_bot
```

> Note: this runs as a **bot**, not your account. If you're thinking about making a userbot that echoes all incoming messages across your chats and groups - please don't. You'd end up auto-replying to everything in every group you're in, which is pretty much a guaranteed way to get reported or rate limited. Bots are the right tool for echoing.

If you want more control over which messages this bot responds to, see [`filters_showcase`](#filters_showcasers).

### `filters_showcase.rs`

Shows how ferogram's filter system works. Filters let you route updates to the right handler based on things like "is this a photo?", "is this a command?", "is this from a group?". They compose with `&`, `|`, and `!` so you can express almost any condition.

```
cargo run --example filters_showcase
```

The filter system is used all over the bot examples. [`order_bot`](#order_botrs) and [`showcase_bot`](#showcase_botrs) both lean on it pretty heavily.

---

## Userbot tools (runs as your account)

These use your personal account via MTProto, not a bot token. They can do things the Bot API simply doesn't expose.

### `dialogs_list.rs`

Prints every chat in your account, newest first, with unread counts. A bot can only see chats it's been added to. Your account sees everything.

```
cargo run --example dialogs_list
```

Once you have a dialog list, [`chat_history`](#chat_historyrs) is a natural next step to read messages from any of those chats.

### `chat_history.rs`

Reads message history from Saved Messages (or any chat you point it at). Fill in `PEER` with a username, phone number, or chat ID. Bots have no way to pull history. This does.

```
cargo run --example chat_history
```

If you want to find specific messages rather than reading all of them in order, check out [`search_messages`](#search_messagesrs).

### `search_messages.rs`

Full-text search across any chat's history by keyword. Runs server-side so it's fast even on huge groups. Set `PEER` and `QUERY` at the top.

```
cargo run --example search_messages
```

### `schedule_message.rs`

Sends a message at a future time. Telegram stores and delivers it server-side, so your process doesn't need to stay running. Set `PEER`, `TEXT`, and `SEND_IN_SECONDS`.

```
cargo run --example schedule_message
```

### `download_media.rs`

Watches for incoming photos and documents and saves them to a `downloads/` folder automatically. Good starting point if you're building a media archiver.

```
cargo run --example download_media
```

### `get_participants.rs`

Lists every member of a group or channel with their role (creator, admin, member, banned). You need to be a member. Set `PEER` to the group username or ID.

```
cargo run --example get_participants
```

If you're doing moderation work, pair this with [`admin_log`](#admin_logrs) to see what actions admins have taken.

### `admin_log.rs`

Reads the admin action log of a supergroup or channel. Shows who banned who, deleted what, changed the title, etc. You need to be an admin. Set `GROUP` to the target.

```
cargo run --example admin_log
```

---

## Bots

These run as a bot. You'll need a token from [@BotFather](https://t.me/BotFather).

All bot examples use `Client::builder()` with `bot_sign_in` or `quick_connect` with an interactive token prompt. Either way the session gets saved so you don't re-auth on every run. See the [quick_connect vs builder](#quick_connect-or-clientbuilder---which-one) section below for the difference.

### `inline_keyboard.rs`

Sends an inline keyboard on `/start` and handles button taps via callback queries. Good intro to how menus and buttons work.

```
cargo run --example inline_keyboard
```

If you want users to be able to trigger your bot from any chat without opening it directly, also look at [`inline_query_bot`](#inline_query_botrs).

### `inline_query_bot.rs`

Lets users type `@your_bot something` anywhere in Telegram and see instant results. Enable inline mode in BotFather first (`/setinline`), then run this.

```
cargo run --example inline_query_bot
```

### `poll_bot.rs`

Creates all four poll types: regular, quiz (with correct answer), multiple choice, and timed. Commands are `/poll`, `/quiz`, `/multi`, `/timed`.

```
cargo run --example poll_bot
```

### `translate_bot.rs`

Reply to any message with `/tr en` (or another language code) and the bot translates it using Telegram's built-in translation API. No external API key needed.

```
cargo run --example translate_bot
```

### `order_bot.rs`

A multi-step conversation bot using FSM (finite state machine). Walks the user through product, quantity, and address before confirming an order. Shows how to handle stateful flows properly.

```
cargo run --example order_bot
```

The routing in here is built on the same filter system shown in [`filters_showcase`](#filters_showcasers).

### `showcase_bot.rs`

The kitchen sink. Basically every feature in one place: commands, callbacks, inline queries, media handling, formatting, and more. Uses `Client::builder()` with all the options. Good reference to copy from when building something real.

```
cargo run --example showcase_bot
```

It covers everything [`inline_keyboard`](#inline_keyboardrs), [`inline_query_bot`](#inline_query_botrs), and [`poll_bot`](#poll_botrs) do individually, and then some.

---

## Session utilities

### `string_session_gen.rs`

Logs in and prints a session string you can save as an env var. Useful when you can't or don't want to write files to disk (a VPS, a container, CI, etc.).

```
cargo run --example string_session_gen
```

Once you have the string, use it with [`serverless_userbot`](#serverless_userbotrs) to verify it works.

### `serverless_userbot.rs`

Loads a session string from the `SESSION_STRING` env var, logs in, and sends a message to your Saved Messages to confirm it worked. That's all it does - a clean sanity check for your string session before you build anything on top of it. Uses `Client::builder()` with `.session_string()` instead of a file.

```
SESSION_STRING="..." cargo run --example serverless_userbot
```

Run [`string_session_gen`](#string_session_genrs) first to get the string.

### `userbot.rs`

The full-featured userbot. Command handler, message editing, file operations, and more. Uses `Client::builder()` with custom device model and app version. This is the "batteries included" reference if you want to see how a real userbot is structured end to end.

```
cargo run --example userbot
```

Or if you want a ready-made userbot without building it yourself, check out [FeroUB](https://github.com/ankit-chaubey/feroub).

---

## quick_connect or Client::builder - which one?

You'll see both used across the examples and it can be a bit confusing at first. Here's the short version:

**`quick_connect`** is the fast path. One line, connects, done. It handles the session file and prompts for login interactively if needed. Use it when you just want to get something running without thinking about connection options.

```rust
let (client, _shutdown) = Client::quick_connect("my.session", API_ID, API_HASH).await?;
```

**`Client::builder()`** is for when you actually need control. Transport type, proxy, device model, language code, resilient reconnect, string sessions - all of that lives here. More code, but more options.

```rust
let (client, _shutdown) = Client::builder()
    .api_id(API_ID)
    .api_hash(API_HASH)
    .transport(TransportKind::Abridged)
    .resilient_connect(true)
    .connect()
    .await?;
```

Most of the simpler examples use `quick_connect`. The heavier ones like [`showcase_bot`](#showcase_botrs) and [`userbot`](#userbotrs) use the builder. Either works for most things - it's just a matter of how much you need to customize.

---

## Quick reference

| Example | Runs as | Connect style | What it does |
|---|---|---|---|
| `hello_self` | user | `quick_connect` | send a message to Saved Messages |
| `echo_bot` | bot | builder | echo every incoming message |
| `filters_showcase` | bot | `quick_connect` | shows filter composition |
| `dialogs_list` | user | `quick_connect` | list all chats with unread counts |
| `chat_history` | user | `quick_connect` | read message history from any chat |
| `search_messages` | user | `quick_connect` | full-text search across chat history |
| `schedule_message` | user | `quick_connect` | schedule a message for later |
| `download_media` | user | `quick_connect` | auto-save incoming photos and files |
| `get_participants` | user | `quick_connect` | list group members with roles |
| `admin_log` | user | `quick_connect` | read the admin action log |
| `inline_keyboard` | bot | `quick_connect` | send buttons, handle taps |
| `inline_query_bot` | bot | `quick_connect` | respond to `@bot query` inline |
| `poll_bot` | bot | `quick_connect` | create polls and quizzes |
| `translate_bot` | bot | `quick_connect` | translate messages via Telegram API |
| `order_bot` | bot | `quick_connect` | multi-step FSM conversation |
| `showcase_bot` | bot | builder | everything in one place |
| `string_session_gen` | user | `quick_connect` | generate a session string |
| `serverless_userbot` | user | builder + string session | verify string session, send to self |
| `userbot` | user | builder | full-featured userbot reference |
