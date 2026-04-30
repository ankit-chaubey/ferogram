# Features

Everything ferogram supports, sourced directly from the codebase.

---

## Authentication

- Phone number login: `request_login_code`, `sign_in`
- Two-factor auth (SRP): `check_password`
- Bot token login: `bot_sign_in`
- QR code login: `export_login_token`, `check_qr_login`
- Check auth status: `is_authorized`
- Sign out: `sign_out`
- List active sessions: `get_authorizations`
- Terminate a specific session: `terminate_session`
- Save session to disk: `save_session`
- Export session as a portable base64 string: `export_session_string`

---

## Connection and Transport

- MTProto Abridged, Obfuscated, and FakeTLS transports
- Transport probing: races multiple transports, uses the first that connects (`probe_transport` config)
- SOCKS5 proxy with optional username/password (`Socks5Config`)
- MTProxy via `t.me/proxy?...` link: `proxy_link`
- MTProxy with explicit host/port/secret: `proxy`
- DNS-over-HTTPS resolver with TTL-based caching, supports Cloudflare and Google
- Telegram special-config fallback via Firebase when TCP is blocked
- Resilient connect mode: chains DoH + special-config on TCP failure (`resilient_connect` config)
- IPv6 DC address support, opt-in (`allow_ipv6`)
- Manual DC address override (`dc_addr`)
- Configurable `ConnectionRestartPolicy` for reconnect behavior
- Configurable `RetryPolicy` for flood-wait and RPC error handling
- `NoRetries` and `AutoSleep` built-in retry policies
- `FLOOD_WAIT` handled automatically by all high-level methods
- Graceful shutdown: `shutdown.cancel()`
- Immediate disconnect: `client.disconnect()`
- Manual network restore signal: `signal_network_restored`
- `InitConnection` fields: device model, system version, app version, system lang code, lang pack, lang code

---

## Session Backends

- `BinaryFileBackend`: single file on disk, default
- `InMemoryBackend`: no persistence, for tests
- `StringSessionBackend`: portable base64 string, env-var and serverless
- `SqliteBackend`: multi-session local SQLite file (`sqlite-session` feature)
- `LibSqlBackend`: Turso / distributed libSQL (`libsql-session` feature)
- Custom backend: implement `SessionBackend` trait

---

## Updates

Typed async update stream via `client.stream_updates()`. All variants:

- `NewMessage`: new incoming or outgoing message
- `MessageEdited`: message text or media edited
- `MessageDeleted`: one or more messages deleted
- `CallbackQuery`: inline button pressed
- `InlineQuery`: user typed in inline mode
- `InlineSend`: user selected an inline result
- `UserStatus`: user online/offline status changed
- `UserTyping`: typing, uploading, recording indicator in a chat
- `ParticipantUpdate`: user joined, left, or was changed in a group/channel
- `JoinRequest`: join request submitted to a group or channel
- `MessageReaction`: reaction added or removed on a message
- `PollVote`: user voted in a poll
- `BotStopped`: user blocked or unblocked the bot
- `ShippingQuery`: shipping query from a payment
- `PreCheckoutQuery`: pre-checkout query from a payment
- `ChatBoost`: channel boost event
- `Raw`: raw TL update for anything not wrapped above

Update gap recovery with PTS/QTS/channel PTS tracking. Missed updates fetched via `getDifference` and `getChannelDifference` on reconnect. `catch_up: true` config field replays updates from last known state. 30-second watchdog unblocks a stuck diff fetch.

---

## Message Sending

- `send_message(peer, InputMessage)`: send a message
- `send_to_self(InputMessage)`: send to Saved Messages
- `msg.reply(InputMessage)`: quote-reply in the same chat
- `msg.respond(InputMessage)`: send to the same chat without quoting
- `msg.edit(InputMessage)`: edit the message
- `msg.forward_to(peer)` / `msg.forward_to_with(client, peer)`: forward to another chat
- `msg.delete()` / `msg.delete_with(client)`: delete the message
- `msg.pin()` / `msg.pin_with(client)`: pin the message
- `msg.unpin()` / `msg.unpin_with(client)`: unpin the message
- `msg.mark_as_read()` / `msg.mark_as_read_with(client)`: mark as read
- `msg.refetch()` / `msg.refetch_with(client)`: reload message from server
- `msg.get_reply()` / `msg.get_reply_with(client)`: fetch the message being replied to
- Edit message: `edit_message(peer, id, InputMessage)`
- Forward messages: `forward_messages(from_peer, ids, to_peer)`
- Delete messages: `delete_messages(peer, ids)`
- Pin message: `pin_message(peer, id)`
- Unpin message: `unpin_message(peer, id)`
- Unpin all: `unpin_all_messages(peer)`
- Get pinned message: `get_pinned_message(peer)`
- Mark as read: `mark_as_read(peer)`
- Export message link: `export_message_link(peer, id)`
- Get message read participants: `get_message_read_participants(peer, id)`
- Click inline button: `msg.click_button(ButtonFilter)`: by position, label, or callback data
- Click inline button by predicate: `msg.click_button_where(|text, data| ...)`
- Find button position: `msg.find_button(ButtonFilter)` / `msg.find_button_where(predicate)`

---

## InputMessage Options

All send/reply/respond calls accept `InputMessage` for full control:

- `.text(str)` / `.markdown(str)` / `.html(str)`
- `.reply_to(Option<i32>)`: reply-to message ID
- `.silent(bool)`: send without notification
- `.background(bool)`: background send flag
- `.clear_draft(bool)`: clear draft after sending
- `.no_webpage(bool)`: disable link preview
- `.invert_media(bool)`: invert media position
- `.schedule_date(Option<i32>)`: Unix timestamp to schedule delivery
- `.schedule_once_online()`: deliver when recipient comes online
- `.entities(Vec<MessageEntity>)`: manual entity list
- `.reply_markup(ReplyMarkup)`: attach inline or reply keyboard
- `.copy_media(InputMedia)`: attach existing media
- `.clear_media()`: strip media from message

---

## Scheduled Messages

- `get_scheduled_messages(peer)`: list all scheduled messages
- `delete_scheduled_messages(peer, ids)`: delete specific scheduled messages
- `send_scheduled_now(peer, ids)`: send scheduled messages immediately

---

## Drafts

- `save_draft(peer, text)`: save or update a draft
- `sync_drafts()`: trigger a server push of all drafts as update events
- `clear_all_drafts()`: delete all drafts

---

## Message Accessors on `IncomingMessage`

Every received message exposes:

- `text()`, `id()`, `peer_id()`, `sender_id()`, `chat_id()`
- `outgoing()`, `date()`, `edit_date()`, `date_utc()`, `edit_date_utc()`
- `mentioned()`, `silent()`, `post()`, `pinned()`, `noforwards()`
- `from_scheduled()`, `edit_hide()`, `media_unread()`
- `forward_count()`, `view_count()`, `reply_count()`, `reaction_count()`
- `reply_to_message_id()`, `reply_markup()`, `forward_header()`
- `via_bot_id()`, `post_author()`, `grouped_id()` (album ID)
- `media()`, `entities()`, `action()` (service message action)
- `photo()`, `document()`: typed media accessors
- `is_private()`, `is_group()`, `is_channel()`, `is_any_group()`
- `is_bot_command()`, `command()`, `command_args()`, `is_command_named(name)`
- `has_media()`, `has_photo()`, `has_document()`
- `is_forwarded()`, `is_reply()`, `album_id()`
- `sender_user_id()`, `sender_chat_id()`, `sender_user()`
- `restriction_reason()`
- `markdown_text()`: message text with Markdown formatting restored
- `html_text()`: message text with HTML formatting restored

---

## File and Media

**Uploading**

- `upload_file(path)`: sequential upload
- `upload_file_concurrent(path, workers)`: parallel chunked upload
- `upload_stream(reader, file_name, size)`: upload from `AsyncRead`
- `upload_media(peer, InputMedia)`: upload and get a reusable `InputMedia`
- Automatic part size selection based on file size
- Automatic worker count scaling based on file size

**Sending media**

- `send_file(peer, InputMedia, caption)`: send any media type
- `send_album(peer, Vec<InputMedia>)`: grouped media album
- `msg.download_media(path)`: download directly from a message

**Downloading**

- `download_media(media, path)`: download to file
- `download_media_concurrent(media, path)`: parallel chunked download
- CDN download for large files (transparent, no extra API calls needed)
- Automatic DC redirect for cross-DC media

**InputMedia variants**

- `InputMedia::upload_file(path)`: local file
- `InputMedia::upload_stream(...)`: stream
- `InputMedia::document(attributes)`: generic document with custom attributes
- `InputMedia::photo(id, access_hash, ...)`: existing photo by ID
- `InputMedia::geo(lat, long)`: location
- `InputMedia::contact(...)`: contact card
- `InputMedia::poll(Poll)`: poll
- `InputMedia::dice(emoticon)`: animated dice
- `InputMedia::sticker(id, ...)`: sticker

---

## Chats and Peers

- `get_me()`: fetch the current user/bot as `User`
- `get_chat_full(peer)`: full chat info including about, pinned msg, etc.
- `get_user_full(user)`: full user info
- `get_common_chats(user)`: mutual groups with another user
- `set_profile(first_name, last_name, about)`: edit own profile
- `set_username(username)`: change own username
- `set_profile_photo(media)`: set profile photo
- `delete_profile_photos(photo_ids)`: remove one or more profile photos
- `set_online()` / `set_offline()`: set online presence
- `set_emoji_status(status)`: set an emoji status
- `create_group(title, users)`: create a basic group
- `create_channel(title, about)`: create a supergroup or channel
- `delete_channel(peer)`: delete a channel or supergroup
- `delete_chat(chat_id)`: delete a basic group (creator only)
- `edit_chat_title(peer, title)`: rename a chat
- `edit_chat_about(peer, about)`: update description
- `edit_chat_photo(peer, media)`: update chat photo
- `edit_chat_default_banned_rights(peer, rights)`: set group-wide default permissions
- `migrate_chat(peer)`: upgrade a basic group to supergroup
- `leave_chat(peer)`: leave a group or channel
- `join_chat(peer)`: join a chat by username or invite
- `accept_invite_link(link)`: join via invite link
- `archive_chat(peer)` / `unarchive_chat(peer)`: archive control
- `delete_chat_history(peer, ...)`: delete own message history in a chat
- `set_history_ttl(peer, seconds)`: set auto-delete timer
- `toggle_no_forwards(peer, enabled)`: restrict forwarding in a chat
- `set_chat_theme(peer, emoticon)`: set chat theme emoji
- `get_online_count(peer)`: approximate online member count
- `get_linked_channel(peer)`: linked discussion channel or group
- `get_media_group(peer, msg_id)`: fetch all messages in an album
- `get_send_as_peers(peer)`: list peers the user can send as
- `set_default_send_as(peer, send_as)`: set default send-as identity
- `warm_peer_cache_from_dialogs()`: prefetch peer info from dialog list

---

## Contacts

- `get_contacts()`: full contact list with mutual info
- `add_contact(user, first_name, last_name, phone)`: add a contact
- `delete_contacts(users)`: remove contacts
- `import_contacts(contacts)`: bulk import
- `block_user(user)` / `unblock_user(user)`: block/unblock
- `get_blocked_users(...)`: paginated blocked list
- `search_contacts(query)`: local contact search

---

## Participants and Admin

- `get_participants(peer, filter, ...)`: paginated member list with filters
- `get_participants_filtered(peer, filter, ...)`: filtered variant with finer control
- `get_admin_log(peer, ...)`: channel admin log with event-type filters
- `get_chat_administrators(peer)`: list of current admins
- `get_admins_with_invites(peer)`: admins who have active invite links
- `invite_users(peer, users)`: add users to a group
- `kick_participant(peer, user)`: remove a user from a group
- `ban_participant(peer, user)`: ban a user; `ban_participant_until(peer, user, ts)` with expiry
- `promote_participant(peer, user, AdminRights)`: grant admin rights
- `demote_participant(peer, user)`: revoke admin rights
- `set_admin_rights(peer, user, AdminRights)`: set admin permissions
- `set_banned_rights(peer, user, BannedRights)`: set per-user restrictions
- `get_permissions(peer, user)`: fetch a participant's current rights
- `transfer_chat_ownership(peer, user, password)`: transfer group/channel ownership
- `export_invite_link(peer)`: get or generate primary invite link
- `edit_invite_link(peer, link, ...)`: edit an existing invite link
- `revoke_invite_link(peer, link)`: revoke a link
- `delete_invite_link(peer, link)`: delete a revoked link
- `delete_revoked_invite_links(peer)`: bulk delete all revoked links
- `get_invite_links(peer, ...)`: list all invite links
- `get_invite_link_members(peer, link, ...)`: who joined via a link
- `get_admins_with_invites(peer)`: admins with active invite links
- `join_request(peer, user, approve)`: approve or decline a single join request
- `all_join_requests(peer, link, approve)`: bulk approve or decline join requests

---

## Messages Search

- `search(peer, query)`: returns a `SearchBuilder` for searching within a chat; supports filter, offset, limit
- `search_global(query)`: returns a `GlobalSearchBuilder` for searching across all chats
- `get_message_history(peer, ...)`: paginated message history
- `get_messages_by_id(peer, ids)`: fetch specific messages by ID
- `get_replies(peer, msg_id, ...)`: fetch comments under a channel post
- `iter_messages(peer)`: lazy iterator over message history

**Search filters:** `InputMessagesFilter` variants: photos, video, documents, music, voice, round video, URLs, pinned, geo, contacts, mentions, unread mentions

---

## Inline Keyboards and Callbacks

- `InlineKeyboard::new()`: build inline keyboard via `.row(buttons)`, `.into_markup()`
- `ReplyKeyboard::new()`: build reply keyboard via `.row(buttons)`, `.resize()`, `.single_use()`, `.selective()`, `.into_markup()`
- `Button::callback(text, data)`: callback data button
- `Button::url(text, url)`: URL button
- `Button::url_auth(text, url, ...)`: URL with auth
- `Button::switch_inline(text, query)`: switch to inline mode in another chat
- `Button::switch_elsewhere(text, query)`: switch to inline mode in current chat
- `Button::mini_app(text, url)`: mini-app button
- `Button::mini_app_simple(text, url)`: mini-app button (no JS required)
- `Button::game(text)`: game button
- `Button::buy(text)`: payment button
- `Button::request_phone(text)`: request phone number
- `Button::request_geo(text)`: request location
- `Button::request_poll(text)` / `Button::request_quiz(text)`: request poll/quiz
- `Button::copy_text(text, copy_text)`: copy-to-clipboard button
- Answer callback query: `answer_callback_query(id, text, alert, url, cache_time)`
- Answer inline query: `answer_inline_query(id, results, ...)` with all `InputBotInlineResult` variants
- Edit inline message: `edit_inline_message(msg_id, InputMessage)`

---

## Reactions

- `msg.react(reaction)` / `msg.react_with(client, reaction)`: set reaction on a message
- `send_reaction(peer, msg_id, reactions)`: set reactions via participant context
- `get_reactions(peer, msg_id)`: fetch reaction counts and recent reactors
- `iter_reaction_users(peer, msg_id, reaction)`: paginated list of users for a specific reaction
- `set_chat_reactions(peer, reactions)`: configure allowed reactions in a chat
- `send_paid_reaction(peer, msg_id, count)`: send paid (Stars) reaction
- `read_reactions(peer)`: mark reactions as read
- `clear_recent_reactions()`: clear the recent reactions list

---

## Stickers and Emoji

- `get_sticker_set(short_name)`: fetch a sticker set by short name
- `get_all_stickers(hash)`: all installed sticker sets
- `get_custom_emoji_documents(ids)`: fetch documents for custom emoji IDs
- `install_sticker_set(set, archived)` / `uninstall_sticker_set(set)`: manage installed sets

---

## Polls

- `send_poll(peer, Poll)`: send a poll via `InputMedia::poll`
- `send_vote(peer, msg_id, options)`: cast a vote
- `get_poll_results(peer, msg_id)`: fetch vote counts and voters
- `get_poll_votes(peer, msg_id, option, ...)`: paginated voter list per option

---

## Payments

- `send_invoice(peer, InputMedia)`: send an invoice message
- `answer_shipping_query(id, ok, shipping_options, error)`: respond to a shipping query
- `answer_precheckout_query(id, ok, error)`: respond to a pre-checkout query

---

## Games

- `set_game_score(peer, msg_id, user_id, score, ...)`: set a game score
- `get_game_high_scores(peer, msg_id, user_id)`: get high scores

---

## Topics (Forum Mode)

- `create_forum_topic(peer, title, ...)`: create a topic in a forum supergroup
- `edit_forum_topic(peer, topic_id, ...)`: rename or change icon
- `delete_forum_topic_history(peer, topic_id)`: delete a topic and its messages
- `get_forum_topics(peer, ...)`: paginated topic list
- `get_forum_topics_by_id(peer, ids)`: fetch specific topics by ID
- `toggle_forum(peer, enabled)`: enable/disable forum mode for a supergroup

---

## Folders and Dialogs

- `get_dialogs(limit)`: fetch dialog list
- `iter_dialogs()`: lazy iterator over all dialogs
- `get_pinned_dialogs(folder_id)`: pinned chats list
- `pin_dialog(peer)` / `unpin_dialog(peer)`: pin/unpin in dialog list
- `mark_dialog_unread(peer, unread)`: manual unread flag
- `mark_dialog_read(peer)`: clear unread flag
- `delete_dialog(peer)`: remove a dialog from the list

---

## Router and Dispatcher

- `Dispatcher::new()`: create a dispatcher
- `.on_message(filter, handler)`: handle new messages
- `.on_edit(filter, handler)`: handle edited messages
- `.on_message_fsm(filter, State::Variant, handler)`: FSM-gated message handler
- `.on_edit_fsm(filter, State::Variant, handler)`: FSM-gated edit handler
- `.include(router)`: attach a sub-router
- `.middleware(mw)`: register middleware
- `.with_state_storage(storage)`: set FSM state backend
- `.with_key_strategy(strategy)`: set FSM key strategy
- `.dispatch(update)`: process a single update
- `Router::new()`: create a nested router
- `router.scope(filter)`: restrict all routes to a pre-filter
- `router.include(router)`: nest another router
- `Next::run(update)`: pass update to the next handler in chain
- `DispatchError`: structured error with `.msg(str)` and `.wrap(err)` constructors

---

## FSM

- `on_message_fsm(filter, State::Variant, handler)`: handler fires only when user is in that state
- `on_edit_fsm(filter, State::Variant, handler)`: same for edited messages
- `StateKeyStrategy`: `PerUserPerChat` (default), `PerUser`, `PerChat`
- `MemoryStorage`: in-process DashMap-backed storage
- Custom storage: implement `StateStorage` trait
- `state.transition(S)`: move to next state
- `state.clear_state()`: reset to no state
- `state.set_data(field, value)`: attach serde-serializable data
- `state.get_data::<T>(field)`: retrieve typed data
- `state.get_all_data()`: all data as `HashMap<String, Value>`
- `state.clear_data()`: delete data, keep state
- `state.clear_all()`: delete state and all data
- `state.key()`: inspect active state key
- `#[derive(FsmState)]` on enums: generates `as_key() -> String` and `from_key(&str) -> Option<Self>` (unit variants only, from `ferogram-derive`)

---

## Conversations

- `Conversation::new(client, peer)`: open a sequential request-response session
- `conv.ask(text)`: send a message and wait for the next reply
- `conv.get_response()`: wait for the next incoming message
- `conv.wait_click(deadline)`: wait for a callback query click
- `conv.wait_read(deadline)`: wait until messages are read
- `conv.ask_and_wait(text, deadline)`: send and wait with a deadline
- `conv.peer()`: the conversation's peer
- `conv.drain_buffered()`: collect updates buffered during the conversation

---

## Filters

Composable `Filter` trait: `and` (`&`), `or` (`|`), `not` (`!`) combinators on any filter.

Built-in filters: `all`, `none`, `private`, `group`, `channel`, `text`, `media`, `photo`, `document`, `forwarded`, `reply`, `album`, `any_command`, `command(name)`, `text_contains(needle)`, `text_starts_with(prefix)`, `from_user(id)`, `in_chat(id)`, `custom(fn)`

---

## Middleware

- `Middleware` trait: implement `call(update, next)` for custom logic
- `TracingMiddleware`: logs each update at debug level
- `RateLimitMiddleware::new(max_calls, window)`: per-user sliding window rate limit; `tracked_users()` reports active users
- `PanicRecoveryMiddleware`: catches handler panics, logs them, continues dispatch

---

## Text Formatting

- `InputMessage::text(str)`: plain text
- `InputMessage::markdown(str)`: parse Markdown entities at send time
- `InputMessage::html(str)`: parse HTML entities at send time
- `parse_html(str) -> (String, Vec<MessageEntity>)`: built-in HTML parser (`html` feature)
- `generate_html(text, entities) -> String`: render entities back to HTML
- `parse_markdown(str) -> (String, Vec<MessageEntity>)`: Markdown parser
- `generate_markdown(text, entities) -> String`: render entities back to Markdown
- `html5ever`-based HTML parser as a spec-compliant alternative (`html5ever` feature)
- `msg.markdown_text()`: re-renders message text with Markdown formatting applied
- `msg.html_text()`: re-renders message text with HTML formatting applied

---

## Web Pages

- `get_web_page_preview(url)`: fetch Telegram web page preview for a URL

---

## Discussions

- `get_replies(peer, msg_id, ...)`: fetch comments under a channel post
- `get_discussion_message(peer, msg_id)`: get the linked discussion message
- `read_discussion(peer, msg_id, read_max_id)`: mark discussion as read

---

## Translation and Transcription

- `translate_messages(peer, ids, to_lang)`: translate via Telegram's built-in translator
- `transcribe_audio(peer, msg_id)`: voice-to-text transcription
- `toggle_peer_translations(peer, disabled)`: enable/disable translation bar for a chat

---

## Privacy

- `get_privacy(key)`: current rule for a key
- `set_privacy(key, rules)`: set rule for a key

Keys: last seen, profile photo, phone number, bio, birthday, forwards, calls, voice messages, group add, gifts. Audiences: everybody, contacts, close friends, nobody, plus explicit allow/disallow lists.

- `get_notify_settings(peer)`: fetch notification settings for a peer
- `update_notify_settings(peer, settings)`: mute, sound, alerts, etc.

---

## Statistics

- `get_broadcast_stats(peer)`: channel stats: followers, views, shares, message activity
- `get_megagroup_stats(peer)`: supergroup stats: growth, languages, member activity

---

## Bot Configuration

- `set_bot_commands(scope, lang_code, commands)`: set command list scoped by chat type and language
- `delete_bot_commands(scope, lang_code)`: remove commands for a scope
- `set_bot_info(bot, lang_code, name, about, description)`: set bot metadata
- `get_bot_info(bot, lang_code)`: fetch bot name, about, description
- `start_bot(bot, peer, start_param)`: programmatically start a bot with a deep link parameter

---

## Dice and Animated Emoji

- `send_dice(peer, emoticon)`: send animated dice, dart, basketball, football, slots, or bowling

---

## Mini Apps

- `open_mini_app(peer, MiniApp)`: open a Telegram mini-app and return a `MiniAppSession`
- `MiniApp::Main`: bot's main mini-app
- `MiniApp::Url(url)`: mini-app at a specific URL
- `session.prolong()`: keep the session alive

---

## Raw API

- `client.invoke(&req)`: call any TL function against the current DC
- `client.invoke_on_dc(dc_id, &req)`: call any TL function against a specific DC
- Full Layer 224 type coverage via `ferogram::tl`: `tl::types`, `tl::enums`, `tl::functions`

---

## TL Layer Crates

- `ferogram-tl-types`: 2,329 TL definitions generated at build time from Layer 224; binary TL serialization and deserialization on all types
- `ferogram-tl-gen`: build-time code generator from a TL AST to Rust source
- `ferogram-tl-parser`: streaming parser for `.tl` schema files; produces a typed `Definition` AST; internal `TlIterator` for low-memory streaming; parse errors include the failing line

---

## Cryptography (`ferogram-crypto`)

All implemented from scratch, no external crypto service:

- AES-IGE encrypt/decrypt: MTProto message encryption
- AES-CTR: CDN file decryption
- RSA (MTProto RSA-PAD scheme): DH handshake
- SHA-1 and SHA-256
- Diffie-Hellman key generation and server parameter verification
- `ObfCipher`: obfuscated transport key derivation
- `derive_keys`: auth key derivation from DH output
- MTProto 2.0 message framing: sequence numbers, salts, session IDs

---

## MTProto Protocol (`ferogram-mtproto`)

- MTProto 2.0 session with full encryption
- DH handshake on first connect (auth key generation)
- Message acknowledgment and automatic resend
- Persistent connection with keep-alive pings
- Multi-DC connection pool
- Split `ConnectionWriter`/reader with independent locks, avoids blocking under burst load
- Automatic session salt refresh
- Bind temp key handshake for PFS

---

## Cargo Feature Flags

**`ferogram`**

| Flag | Default | Description |
|---|---|---|
| `sqlite-session` | no | `SqliteBackend` via rusqlite |
| `libsql-session` | no | `LibSqlBackend` via libsql (Turso) |
| `html` | no | Built-in HTML entity parser and generator |
| `html5ever` | no | Spec-compliant html5ever HTML parser |
| `derive` | no | `#[derive(FsmState)]` proc-macro |
| `serde` | no | serde on session and config types |
| `parser` | no | Re-export `ferogram-tl-parser` for custom tooling |
| `codegen` | no | Re-export `ferogram-tl-gen` for custom tooling |

**`ferogram-tl-types`**

| Flag | Default | Description |
|---|---|---|
| `tl-api` | yes | Telegram API schema |
| `tl-mtproto` | no | MTProto internal schema |
| `impl-debug` | yes | `#[derive(Debug)]` on all types |
| `impl-from-type` | yes | `From<types::T> for enums::E` |
| `impl-from-enum` | yes | `TryFrom<enums::E> for types::T` |
| `deserializable-functions` | no | `Deserializable` on function types |
| `name-for-id` | no | `name_for_id(u32) -> Option<&'static str>` |
| `impl-serde` | no | serde on all generated types |

---

## Testing and Platform

- `InMemoryBackend` for tests without real credentials
- Integration test suite in `ferogram/tests/integration.rs`
- `cargo test --workspace` runs all crates
- `cargo test --workspace --all-features` covers optional backends
- `Send + Sync` on all public types
- Works on Linux, macOS, Windows
- Works in Termux (Android) with the native Rust toolchain
- Async throughout, built on Tokio
- No blocking mutex in async hot paths
