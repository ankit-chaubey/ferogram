# Features

Everything in this file reflects what is actually in the codebase. Nothing is listed that is not implemented.

---

## Authentication

- Phone number login: `request_login_code`, `sign_in`
- Two-factor auth (SRP): `check_password`
- Bot token login: `bot_sign_in`
- QR code login: `export_login_token`, `check_qr_login`
- Sign out: `sign_out`
- Check auth status: `is_authorized`
- Get current user: `get_me`, `my_id`, `my_id_or_fetch`
- List active sessions: `get_authorizations`
- Terminate a specific session: `terminate_session`
- Save session to disk: `save_session`
- Export session as portable base64: `export_session_string`, `export_native_session_string`

---

## Connection and Transport

- MTProto Abridged, Obfuscated, and FakeTLS transports
- Transport probing: races multiple transports, uses whichever connects first (`probe_transport` config)
- SOCKS5 proxy with optional username/password (`socks5`, `socks5_auth`)
- MTProxy via `t.me/proxy?...` link: `proxy_link`
- MTProxy with explicit host/port/secret: `proxy`
- DNS-over-HTTPS resolver with TTL-based caching, supports Cloudflare and Google
- Telegram special-config fallback via Firebase when TCP is blocked
- Resilient connect mode: chains DoH and special-config on TCP failure (`resilient_connect` config)
- IPv6 DC address support, opt-in (`allow_ipv6`)
- Manual DC address override (`dc_addr`)
- Configurable `ConnectionRestartPolicy` for reconnect behavior
- Configurable `RetryPolicy` for flood-wait and RPC error handling
- `NoRetries` and `AutoSleep` built-in retry policies
- `FLOOD_WAIT` handled automatically by all high-level methods
- Graceful shutdown via `ShutdownToken`
- Immediate disconnect: `client.disconnect()`
- Manual network restore signal: `signal_network_restored`
- `InitConnection` fields configurable: device model, system version, app version, lang code, lang pack

---

## Session Backends

- `BinaryFileBackend`: single file on disk, default
- `InMemoryBackend`: no persistence, useful for tests
- `StringSessionBackend`: portable base64 string for serverless and env-var use
- `SqliteBackend`: local SQLite file (`sqlite-session` feature)
- `LibSqlBackend`: Turso / distributed libSQL (`libsql-session` feature)
- Custom backend: implement the `SessionBackend` trait

---

## Updates

Typed async update stream via `client.stream_updates()`. Available variants:

- `NewMessage`: incoming or outgoing message
- `MessageEdited`: message text or media was edited
- `MessageDeleted`: one or more messages were deleted
- `CallbackQuery`: inline button pressed
- `InlineQuery`: user typed in inline mode
- `InlineSend`: user selected an inline result
- `UserStatus`: user online/offline status changed
- `UserTyping`: typing, uploading, or recording indicator in a chat
- `ParticipantUpdate`: user joined, left, or changed role in a group or channel
- `JoinRequest`: join request submitted to a group or channel
- `MessageReaction`: reaction added or removed on a message (bots only)
- `PollVote`: user voted in a poll (bots that sent the poll only)
- `BotStopped`: user blocked or unblocked the bot
- `ShippingQuery`: shipping query from a payment (bots only)
- `PreCheckoutQuery`: pre-checkout query from a payment (bots only)
- `ChatBoost`: channel boost event (bots managing the channel only)
- `GuestChatQuery`: user invited the bot into a guest-chat context (bots only)
- `Raw`: raw TL update bytes for anything not covered above

Update gap recovery with PTS/QTS/channel PTS tracking. Missed updates fetched via `getDifference` and `getChannelDifference` on reconnect. `catch_up: true` config field replays updates from last known state. 30-second watchdog unblocks a stuck diff fetch.

---

## Message Sending

- `send_message(peer, InputMessage)`: send a message to any peer
- `send_to_self(InputMessage)`: send to Saved Messages
- `edit_message(peer, id, InputMessage)`: edit a message
- `forward_messages(from_peer, ids, to_peer)`: forward messages
- `delete_messages(peer, ids)`: delete messages
- `pin_message(peer, id)`: pin a message
- `unpin_all_messages(peer)`: unpin everything in a chat
- `get_pinned_message(peer)`: fetch the current pinned message
- `mark_read(peer)`: mark all messages as read
- `export_message_link(peer, id)`: get a public link to a message
- `get_message_read_participants(peer, id)`: who has read the message
- `send_chat_action(peer, action)`: send a typing or upload indicator
- `send_dice(peer, emoticon)`: send an animated dice

Convenience methods on `IncomingMessage` (require the message to carry a client reference):

- `msg.reply(InputMessage)`: quote-reply in the same chat
- `msg.respond(InputMessage)`: send to the same chat without quoting
- `msg.edit(InputMessage)`: edit the message
- `msg.delete()` / `msg.delete_with(client)`: delete the message
- `msg.pin()` / `msg.pin_with(client)`: pin the message
- `msg.unpin()` / `msg.unpin_with(client)`: unpin the message
- `msg.mark_read()` / `msg.mark_read_with(client)`: mark as read
- `msg.forward_to(peer)` / `msg.forward_to_with(client, peer)`: forward to another chat
- `msg.refetch()` / `msg.refetch_with(client)`: reload message from server
- `msg.get_reply()` / `msg.get_reply_with(client)`: fetch the message being replied to
- `msg.react(reaction)` / `msg.react_with(client, reaction)`: set a reaction on the message
- `msg.click_button(ButtonFilter)`: click an inline button by position, label, or data
- `msg.click_button_where(predicate)`: click an inline button by predicate

---

## InputMessage Options

All send and edit calls accept `InputMessage`:

- `.text(str)` / `.markdown(str)` / `.html(str)`
- `.reply_to(Option<i32>)`: quote-reply by message ID
- `.silent(bool)`: send without a notification sound
- `.background(bool)`: background send flag
- `.clear_draft(bool)`: clear draft after sending
- `.no_webpage(bool)`: disable link preview
- `.invert_media(bool)`: invert media position in message
- `.schedule_date(Option<i32>)`: Unix timestamp to schedule delivery
- `.schedule_once_online()`: deliver when recipient next comes online
- `.entities(Vec<MessageEntity>)`: manual entity list
- `.reply_markup(ReplyMarkup)`: attach inline or reply keyboard
- `.copy_media(InputMedia)`: attach existing media by reference
- `.clear_media()`: strip media from the message

---

## Scheduled Messages

- `get_scheduled_messages(peer)`: list all scheduled messages in a chat
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
- `via_bot_id()`, `post_author()`, `grouped_id()`
- `media()`, `entities()`, `action()`
- `photo()`, `document()`: typed media accessors
- `is_private()`, `is_group()`, `is_channel()`, `is_any_group()`
- `is_bot_command()`, `command()`, `command_args()`, `is_command_named(name)`
- `has_media()`, `has_photo()`, `has_document()`
- `is_forwarded()`, `is_reply()`, `album_id()`
- `sender_user_id()`, `sender_chat_id()`, `sender_user()`
- `restriction_reason()`
- `markdown_text()`: message text with Markdown formatting restored
- `html_text()`: message text with HTML formatting restored
- `channel_kind()`: async lookup of `ChannelKind` (Broadcast / Megagroup / Gigagroup)
- `is_megagroup()`, `is_broadcast()`, `is_gigagroup()`: async channel-type helpers

---

## Files and Transfers

**Uploading**

- `upload_file(path, handle)`: upload a file from disk; uses concurrent workers for large files automatically
- `upload(source, name, handle)`: upload from any `AsyncRead` source
- `upload_sequential(path, handle)`: sequential single-chunk path, constant RAM regardless of file size
- `upload_resumable(data, name, handle, on_progress)`: resumable upload with checkpoint; resumes after crash or cancel (`experimental` feature)
- `upload_exp(path, handle, TransferConfig)`: manual worker count and chunk size (`experimental` feature)

**Downloading**

- `download(media, dest, handle)`: stream to any `AsyncWrite` (buffer, file, socket)
- `download_file(media, path, handle)`: stream directly to a file on disk
- `iter_download(media)`: lazy chunk iterator; process bytes as they arrive
- `download_resumable(media, dest, handle, on_progress)`: resumable download with checkpoint (`experimental` feature)
- `download_exp(media, dest, handle, TransferConfig)`: manual worker count and chunk size (`experimental` feature)

**Transfer control**

- `TransferHandle`: clone freely; call `pause()`, `resume()`, `cancel()` from any task
- `TransferProgress`: snapshot of `done`, `total`, `elapsed_ms`; helpers `percent()`, `speed_bps()`, `speed_human()`, `eta_secs()`, `bytes_human()`

**Media groups**

- `get_media_group(peer, msg_id)`: fetch all messages belonging to an album

**CDN**

- CDN DC downloads handled transparently; no extra API calls needed from the caller

---

## Peer Resolution

Every `Client` method that targets a peer accepts any of the following without pre-resolution:

- `"@username"` or `"username"`
- `"me"` or `"self"`
- E.164 phone number: `"+12025551234"`
- `t.me/<username>` URL
- Invite link: `t.me/+HASH`, `t.me/joinchat/HASH`, `tg://join?invite=HASH`
- `i64` or `i32` numeric ID
- `tl::enums::Peer` or `tl::enums::InputPeer` (zero-cost, no RPC)

Resolution is cache-first. An RPC is only made on a genuine cache miss.

Manual resolution:

- `resolve(peer)`: resolves any peer type to a `tl::enums::Peer`
- `resolve_to_input_peer(peer)`: resolves to `tl::enums::InputPeer` with access hash
- `join_link(link)`: join and resolve an invite link
- `check_invite(link)`: inspect an invite link without joining
- `warm_peer_cache_from_dialogs()`: prefetch peer info from dialog list
- `cache_user(user)` / `cache_entities(users, chats)`: feed `User`/`Chat` objects from a hand-rolled RPC response into the peer cache, same as built-in methods do internally

Peer ID helpers via `PeerExt` and `OptionPeerExt`:

- `peer.bare_id()`: extract numeric ID from a `tl::enums::Peer` without matching

---

## Chats and Channels

- `get_chat_full(peer)`: full chat info
- `get_user_full(user)`: full user info
- `get_users_by_id(ids)`: fetch multiple users by ID
- `get_user_from_message(peer, msg_id)`: resolve sender from a message
- `get_common_chats(user)`: mutual groups with another user
- `create_group(title, users)`: create a basic group
- `create_channel(title, about)`: create a supergroup or channel
- `delete_chat(peer)`: delete a channel or supergroup
- `leave_chat(peer)`: leave a group or channel
- `join_chat(peer)`: join by username or peer reference
- `migrate_chat(chat_id)`: upgrade a basic group to supergroup
- `delete_chat_history(peer, ...)`: delete own message history
- `set_history_ttl(peer, seconds)`: set auto-delete timer
- `edit_chat_default_banned_rights(peer, rights)`: set group-wide default permissions
- `toggle_no_forwards(peer, enabled)`: restrict forwarding
- `set_chat_theme(peer, emoticon)`: set chat theme emoji
- `set_chat_reactions(peer, reactions)`: configure allowed reactions
- `get_online_count(peer)`: approximate online member count
- `get_linked_channel(peer)`: linked discussion channel or group
- `get_admin_log(peer, ...)`: channel admin log with event-type filters
- `get_send_as_peers(peer)`: list peers the user can send as
- `set_default_send_as(peer, send_as)`: set default send-as identity

---

## Profile

- `set_profile(peer)`: fluent builder for editing own or chat profile
- `delete_profile_photos(photo_ids)`: remove profile photos
- `set_presence(bool)`: set online or offline

---

## Contacts

- `get_contacts()`: full contact list
- `add_contact(user, first_name, last_name, phone)`: add a contact
- `delete_contacts(users)`: remove contacts
- `import_contacts(contacts)`: bulk import
- `block(user, true/false)`: block or unblock a user
- `get_blocked_users(...)`: paginated blocked list
- `search_contacts(query)`: local contact search

---

## Participants and Admin

- `get_participants(peer, filter, ...)`: paginated member list
- `get_participants_filtered(peer, filter, ...)`: filtered variant
- `get_chat_administrators(peer)`: current admin list
- `get_permissions(peer, user)`: fetch a participant's current rights
- `invite_users(peer, users)`: add users to a group
- `kick(peer, user)`: remove a user
- `ban(peer, user, until)`: ban permanently or until a timestamp
- `restrict(peer, user, BannedRights)`: set per-user restrictions
- `set_admin(peer, user, AdminRights)`: grant or modify admin rights
- `transfer_chat_ownership(peer, user, password)`: transfer ownership
- `get_profile_photos(peer)`: fetch profile photos
- `iter_profile_photos(peer)`: lazy iterator over profile photos
- `search_peer(query)`: search for peers by name fragment
- `send_reaction(peer, msg_id, reactions)`: set reactions on a message

`AdminRightsBuilder` and `BannedRightsBuilder` cover all Telegram permission flags including `manage_topics`, `send_reactions`, and anonymous posting.

---

## Invite Links

- `export_invite_link(peer)`: get or generate the primary invite link
- `revoke_invite_link(peer, link)`: revoke a link
- `edit_invite_link(peer, link, ...)`: edit title, expiry, or usage limit
- `delete_invite_link(peer, link)`: delete a revoked link
- `delete_revoked_invite_links(peer)`: bulk delete all revoked links
- `get_invite_links(peer, ...)`: list all invite links
- `get_invite_link_members(peer, link, ...)`: who joined via a link
- `get_admins_with_invites(peer)`: admins with active invite links
- `join_request(peer, user, approve)`: approve or decline a single join request
- `all_join_requests(peer, link, approve)`: bulk approve or decline

---

## Dialogs

- `get_dialogs(limit)`: fetch dialog list
- `iter_dialogs()`: lazy iterator over all dialogs
- `get_pinned_dialogs(folder_id)`: pinned chats
- `pin_dialog(peer, bool)`: pin or unpin
- `mark_dialog_unread(peer, bool)`: toggle unread flag
- `mark_read(peer)`: mark as read
- `delete_dialog(peer)`: remove from dialog list
- `clear_mentions(peer)`: clear unread mention indicators
- `archive(peer, bool)`: archive or unarchive
- `sync_drafts()`: trigger draft sync
- `clear_all_drafts()`: delete all drafts

---

## Message Search

- `search(peer, query)`: `SearchBuilder` for searching within a chat; supports filter, offset, limit
- `search_global(query)`: `GlobalSearchBuilder` for searching across all chats
- `get_message_history(peer, ...)`: paginated message history, returns `MessagePage` (messages + count + offset_id_offset)
- `get_messages(peer, ids)`: fetch specific messages by ID
- `get_replies(peer, msg_id, ...)`: fetch comments under a channel post
- `iter_messages(peer)`: lazy iterator over message history

---

## Discussions

- `get_replies(peer, msg_id, ...)`: comments under a channel post
- `get_discussion_message(peer, msg_id)`: linked discussion message
- `read_discussion(peer, msg_id, read_max_id)`: mark discussion as read

---

## Inline Keyboards and Callbacks

- `InlineKeyboard::new()` via `.row(buttons)`, `.into_markup()`
- `ReplyKeyboard::new()` via `.row(buttons)`, `.resize()`, `.single_use()`, `.selective()`, `.into_markup()`
- `Button` variants: `callback`, `url`, `url_auth`, `switch_inline`, `switch_elsewhere`, `mini_app`, `mini_app_simple`, `game`, `buy`, `request_phone`, `request_geo`, `request_poll`, `request_quiz`, `copy_text`
- `answer_callback_query(id, text, alert, url, cache_time)`: respond to a button click
- `answer_inline_query(id, results, ...)`: respond to an inline query
- `edit_inline_message(msg_id, InputMessage)`: edit an inline message

---

## Reactions

- `get_reactions(peer, msg_id)`: fetch reaction counts and recent reactors
- `delete_reaction(peer, msg_id, participant)`: remove a specific user's reaction
- `iter_reaction_users(peer, msg_id, reaction)`: paginated list of users for a reaction
- `set_chat_reactions(peer, reactions)`: configure allowed reactions in a chat
- `send_paid_reaction(peer, msg_id, count)`: send a paid Stars reaction
- `read_reactions(peer)`: mark reactions as read
- `clear_recent_reactions()`: clear the recent reactions list

---

## Stickers and Custom Emoji

- `get_sticker_set(short_name)`: fetch a sticker set by short name
- `get_all_stickers(hash)`: all installed sticker sets
- `get_custom_emoji_documents(ids)`: fetch documents for custom emoji IDs
- `toggle_stickers(set, bool)`: install or uninstall a sticker set

---

## Polls

- `send_poll(peer, PollBuilder)`: send a poll via the `PollBuilder` fluent builder
- `send_vote(peer, msg_id, options)`: cast a vote
- `poll_results(peer, msg_id)`: fetch vote counts
- `get_poll_votes(peer, msg_id, option, ...)`: paginated voter list per option

---

## Payments

- `send_invoice(peer, InputMedia)`: send an invoice message
- `answer_shipping_query(id, ok, options, error)`: respond to a shipping query
- `answer_precheckout_query(id, ok, error)`: respond to a pre-checkout query

---

## Games

- `set_game_score(peer, msg_id, user_id, score, ...)`: set a user's game score
- `get_game_high_scores(peer, msg_id, user_id)`: get high scores for a game

---

## Forum Topics

- `create_forum_topic(peer, title, ...)`: create a topic in a forum supergroup
- `edit_forum_topic(peer, topic_id, ...)`: rename or change icon
- `delete_forum_topic_history(peer, topic_id)`: delete topic and its messages
- `get_forum_topics(peer, ...)`: paginated topic list
- `get_forum_topics_by_id(peer, ids)`: fetch topics by ID
- `toggle_forum(peer, enabled)`: enable or disable forum mode

---

## Bot Configuration

- `set_bot_commands(scope, lang_code, commands)`: set command list by scope and language
- `delete_bot_commands(scope, lang_code)`: remove commands for a scope
- `set_bot_info(bot, lang_code, name, about, description)`: set bot metadata
- `get_bot_info(bot, lang_code)`: fetch bot name, about, description
- `start_bot(bot, peer, start_param)`: start a bot with a deep-link parameter

---

## Mini Apps

- `open_mini_app(peer, MiniApp)`: open a Telegram mini app, returns a `MiniAppSession`
- `MiniApp::Main`: bot's main mini app
- `MiniApp::Url(url)`: mini app at a specific URL
- `session.prolong()`: keep the session alive

---

## Translation and Transcription

- `translate_messages(peer, ids, to_lang)`: translate via Telegram's built-in translator
- `transcribe_audio(peer, msg_id)`: voice-to-text transcription
- `toggle_peer_translations(peer, disabled)`: enable or disable the translation bar

---

## Privacy

- `get_privacy(key)`: current privacy rule for a key
- `set_privacy(key, rules)`: set rule for a key

Keys: last seen, profile photo, phone number, bio, birthday, forwards, calls, voice messages, group add, gifts. Audiences: everybody, contacts, close friends, nobody, plus explicit allow/disallow lists.

- `get_notify_settings(peer)`: fetch notification settings
- `update_notify_settings(peer, settings)`: mute, sound, alerts

---

## Statistics

- `stats(peer)`: channel stats (followers, views, shares) or supergroup stats (growth, languages, activity)
- `get_online_count(peer)`: approximate current online member count

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
- `Router::new()` with `.scope(filter)` and `.include(router)` for nesting

---

## FSM

Requires the `fsm` feature flag.

- `on_message_fsm(filter, State::Variant, handler)`: handler fires only when user is in that state
- `StateKeyStrategy`: `PerUserPerChat` (default), `PerUser`, `PerChat`
- `MemoryStorage`: in-process DashMap-backed state storage
- Custom storage: implement the `StateStorage` trait
- Context methods: `state.transition(S)`, `state.clear_state()`, `state.set_data(field, value)`, `state.get_data::<T>(field)`, `state.get_all_data()`, `state.clear_data()`, `state.clear_all()`, `state.key()`
- `#[derive(FsmState)]`: generates `as_key()` and `from_key()` on unit-variant enums (`derive` feature)

---

## Conversations

- `Conversation::new(client, peer)`: open a sequential request-response session
- `conv.ask(text)`: send a message and wait for the next reply
- `conv.get_response()`: wait for the next incoming message
- `conv.wait_click(deadline)`: wait for a callback query
- `conv.wait_read(deadline)`: wait until messages are read
- `conv.ask_and_wait(text, deadline)`: send and wait with a deadline

---

## Filters

All filters implement `and` (`&`), `or` (`|`), `not` (`!`) combinators.

Built-in: `all`, `none`, `private`, `group`, `channel`, `text`, `media`, `photo`, `document`, `forwarded`, `reply`, `album`, `any_command`, `command(name)`, `text_contains(needle)`, `text_starts_with(prefix)`, `from_user(id)`, `in_chat(id)`, `custom(fn)`

---

## Middleware

- `TracingMiddleware`: logs each update at debug level
- `RateLimitMiddleware::new(max_calls, window)`: per-user sliding window rate limit
- `PanicRecoveryMiddleware`: catches handler panics, logs them, continues dispatch
- Custom middleware: implement the `Middleware` trait

---

## Text Formatting

- `parse_html(str)`: parse HTML entities into `(String, Vec<MessageEntity>)` (`html` feature)
- `generate_html(text, entities)`: render entities back to HTML
- `parse_markdown(str)`: Markdown parser
- `generate_markdown(text, entities)`: render entities back to Markdown
- `html5ever`-based parser as a spec-compliant alternative (`html5ever` feature)
- `msg.markdown_text()` / `msg.html_text()`: re-render received message text with formatting

---

## Raw API

- `client.invoke(&req)`: call any TL function against the current DC
- `client.invoke_on_dc(dc_id, &req)`: call any TL function against a specific DC
- Full Layer 227 type coverage via `ferogram::tl`: `tl::types`, `tl::enums`, `tl::functions`

---

## Cargo Feature Flags

| Flag | Default | Description |
|---|---|---|
| `parsers` | yes | HTML and Markdown entity parsers |
| `html` | yes | HTML entity parser and generator |
| `derive` | no | `#[derive(FsmState)]` proc-macro |
| `fsm` | no | FSM state machine support |
| `sqlite-session` | no | `SqliteBackend` via rusqlite |
| `libsql-session` | no | `LibSqlBackend` via libsql (Turso) |
| `experimental` | no | Resumable transfers, `upload_exp`, `download_exp` |
| `serde` | no | serde on session and config types |
| `html5ever` | no | Spec-compliant html5ever HTML parser |
| `parser` | no | Re-export `ferogram-tl-parser` for custom tooling |
| `codegen` | no | Re-export `ferogram-tl-gen` for custom tooling |

---

## Testing and Platform

- `InMemoryBackend` for tests that do not need real credentials
- Integration test suite in `ferogram/tests/`
- `Send + Sync` on all public types
- Async throughout, built on Tokio
- Runs on Linux, macOS, Windows, and Termux (Android)
- No blocking mutex in async hot paths
