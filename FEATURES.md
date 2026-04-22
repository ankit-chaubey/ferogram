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
- Transport probing: races multiple transports, uses the first that connects (`probe_transport`)
- SOCKS5 proxy with optional username/password (`Socks5Config`)
- MTProxy via `t.me/proxy?...` link: `proxy_link`
- MTProxy with explicit host/port/secret: `proxy`
- DNS-over-HTTPS resolver with TTL-based caching, supports Cloudflare and Google
- Telegram special-config fallback via Firebase when TCP is blocked
- Resilient connect mode: chains DoH + special-config on TCP failure (`resilient_connect`)
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

- `BinaryFileBackend` - single file on disk, default
- `InMemoryBackend` - no persistence, for tests
- `StringSessionBackend` - portable base64 string, env-var and serverless
- `SqliteBackend` - multi-session local SQLite file (`sqlite-session` feature)
- `LibSqlBackend` - Turso / distributed libSQL (`libsql-session` feature)
- Custom backend: implement `SessionBackend` trait

---

## Updates

Typed async update stream via `client.stream_updates()`. All variants:

- `NewMessage` - new incoming or outgoing message
- `MessageEdited` - message text or media edited
- `MessageDeleted` - one or more messages deleted
- `CallbackQuery` - inline button pressed
- `InlineQuery` - user typed in inline mode
- `InlineSend` - user selected an inline result
- `UserStatus` - user online/offline status changed
- `UserTyping` - typing, uploading, recording indicator in a chat
- `ParticipantUpdate` - user joined, left, or was changed in a group/channel
- `JoinRequest` - join request submitted to a group or channel
- `MessageReaction` - reaction added or removed on a message
- `PollVote` - user voted in a poll
- `BotStopped` - user blocked or unblocked the bot
- `ShippingQuery` - shipping query from a payment
- `PreCheckoutQuery` - pre-checkout query from a payment
- `ChatBoost` - channel boost event
- `Raw` - raw TL update for anything not wrapped above

Update gap recovery with PTS/QTS/channel PTS tracking. Missed updates fetched via `getDifference` and `getChannelDifference` on reconnect. `catch_up(true)` replays updates from last known state. 30-second watchdog unblocks a stuck diff fetch.

---

## Message Sending

- `send_message(peer, text)` - plain text
- `send_message_to_peer` - with explicit peer
- `send_message_to_peer_ex` - full `InputMessage` control
- `send_to_self` - send to Saved Messages
- `msg.reply(text)` / `msg.reply_with(client, text)`
- `msg.reply_ex(InputMessage)` - reply with full options
- `msg.respond(text)` - respond without quoting
- `msg.respond_ex(InputMessage)` - respond with full options
- Edit message: `edit_message`, `msg.edit(text)`, `msg.edit_with`
- Forward messages: `forward_messages`, `forward_messages_returning`, `msg.forward_to`
- Delete messages: `delete_messages`, `msg.delete`
- Pin message: `pin_message`, `msg.pin`
- Unpin message: `unpin_message`, `msg.unpin`
- Unpin all: `unpin_all_messages`
- Get pinned message: `get_pinned_message`
- Get reply target: `get_reply_to_message`, `msg.get_reply`
- Refetch message from server: `msg.refetch`
- Mark as read: `mark_as_read`, `msg.mark_as_read`
- Export message link: `export_message_link`
- Get message read participants: `get_message_read_participants`

---

## InputMessage Options

All send/reply/respond calls accept `InputMessage` for full control:

- `.text(str)` / `.markdown(str)` / `.html(str)`
- `.reply_to(Option<i32>)` - reply-to message ID
- `.silent(bool)` - send without notification
- `.background(bool)` - background send flag
- `.clear_draft(bool)` - clear draft after sending
- `.no_webpage(bool)` - disable link preview
- `.invert_media(bool)` - invert media position
- `.schedule_date(Option<i32>)` - Unix timestamp to schedule delivery
- `.schedule_once_online()` - deliver when recipient comes online
- `.entities(Vec<MessageEntity>)` - manual entity list
- `.reply_markup(ReplyMarkup)` - attach inline or reply keyboard
- `.keyboard(impl Into<ReplyMarkup>)` - shorthand for keyboard
- `.copy_media(InputMedia)` - attach existing media
- `.clear_media()` - strip media from message

---

## Scheduled Messages

- `get_scheduled_messages(peer)` - list all scheduled messages
- `delete_scheduled_messages(peer, ids)` - delete specific scheduled messages
- `send_scheduled_now(peer, ids)` - send scheduled messages immediately

---

## Drafts

- `save_draft(peer, InputMessage)` - save or update a draft
- `get_all_drafts()` - fetch all drafts
- `clear_all_drafts()` - delete all drafts

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
- `photo()`, `document()` - typed media accessors
- `is_private()`, `is_group()`, `is_channel()`, `is_any_group()`
- `is_bot_command()`, `command()`, `command_args()`, `is_command_named(name)`
- `has_media()`, `has_photo()`, `has_document()`
- `is_forwarded()`, `is_reply()`, `album_id()`
- `sender_user_id()`, `sender_chat_id()`, `sender_user()`
- `restriction_reason()`
- `markdown_text()` - message text with Markdown formatting restored
- `html_text()` - message text with HTML formatting restored

---

## File and Media

**Uploading**

- `upload_file(path)` - sequential upload
- `upload_file_concurrent(path, workers)` - parallel chunked upload
- `upload_stream(reader, file_name, size)` - upload from `AsyncRead`
- `upload_media(peer, InputMedia)` - upload and get a reusable `InputMedia`
- Automatic part size selection based on file size
- Automatic worker count scaling based on file size

**Sending media**

- `send_file(peer, InputMedia, caption)` - send any media type
- `send_album(peer, Vec<InputMedia>)` - grouped media album
- `msg.download_media(path)` - download directly from a message

**Downloading**

- `download_media(location)` - sequential download to `Vec<u8>`
- `download_media_on_dc(location, dc_id)` - force specific DC
- `download_media_concurrent(location)` - parallel chunked download
- `download_media_concurrent_on_dc(location, dc_id)`
- `download_media_to_file(location, path)`
- `download_media_to_file_on_dc(location, path, dc_id)`
- `iter_download(location)` - streaming iterator for manual chunk handling
- `iter_download_on_dc(location, dc_id)`
- `client.download(item)` - download anything implementing `Downloadable`
- `media_dc_addr(dc_id)` / `best_media_dc_addr()` - DC address lookup

**CDN downloads**

- `CdnDownloader::connect` - open CDN session for large file downloads
- `download_chunk_raw` - single raw chunk from CDN
- `download_all` - full CDN download with AES-CTR decryption and hash verification
- `download_all_with_reupload` - CDN download with server-side reupload callback

**Media types**

- `Photo` - id, access_hash, date, has_stickers, largest_thumb_type, download_location
- `Document` - id, access_hash, date, mime_type, size, file_name, is_animated, download_location
- `Sticker` - from_document, emoji, is_video, id, mime_type, download_location
- `get_media_group(peer, msg_id)` - fetch all messages in an album

---

## Peers and Peer Resolution

- `resolve_peer(username_or_link)` - resolve any string to `InputPeer`
- `resolve_username(username)` - resolve username to typed peer
- `resolve_to_input_peer(PeerRef)` - convert `PeerRef` to `InputPeer`
- `get_me()` - fetch the logged-in user
- `get_users_by_id(ids)` - batch fetch users by ID
- `get_user_full(peer)` - full user info including bio, common chats count
- `get_chat_full(peer)` - full chat/channel info
- `get_online_count(peer)` - online member count
- `PeerRef` - flexible argument: accepts username, ID, phone, `t.me` link, or raw TL peer

**`User` accessors**

`id`, `access_hash`, `first_name`, `last_name`, `username`, `usernames`, `phone`, `verified`, `bot`, `deleted`, `blocked`, `premium`, `full_name`, `status`, `photo`, `is_self`, `contact`, `mutual_contact`, `scam`, `restricted`, `bot_privacy`, `bot_supports_chats`, `bot_inline_geo`, `support`, `lang_code`, `restriction_reason`, `bot_inline_placeholder`

**`Group` accessors**

`id`, `title`, `participants_count`, `creator`, `migrated_to`

**`Channel` accessors**

`id`, `access_hash`, `title`, `username`, `usernames`, `megagroup`, `broadcast`, `verified`, `restricted`, `signatures`, `participants_count`, `kind` (Megagroup/Broadcast/Gigagroup), `photo`, `admin_rights`, `restriction_reason`

---

## Dialogs and History

- `get_dialogs(limit)` - fetch dialogs list
- `iter_dialogs()` - paginated `DialogIter` (total count available)
- `iter_messages(peer)` - paginated `MessageIter` for message history (total count available)
- `get_messages(peer, ids)` - fetch specific messages by ID list
- `get_messages_by_id(peer, ids)` - same with batch handling
- `count_channels()` - count all joined channels
- `delete_dialog(peer)` - delete a dialog
- `delete_chat_history(peer)` - delete full chat history
- `mark_as_read(peer)` - mark all messages as read
- `clear_mentions(peer)` - clear all unread mentions
- `pin_dialog(peer)` - pin a dialog
- `unpin_dialog(peer)` - unpin a dialog
- `archive_chat(peer)` - archive a chat
- `unarchive_chat(peer)` - unarchive a chat
- `get_pinned_dialogs(folder_id)` - list pinned dialogs
- `mark_dialog_unread(peer, unread)` - toggle unread mark

`Dialog` accessors: `title`, `peer`, `unread_count`, `top_message`

---

## Search

**Per-chat search** via `SearchBuilder` (`.search(peer, query)`):

- `min_date`, `max_date` - date range
- `filter(MessagesFilter)` - message type (photos, videos, documents, audio, links, etc.)
- `limit`, `offset_id`, `add_offset`, `max_id`, `min_id`
- `sent_by_self()` - messages from self only
- `from_peer(InputPeer)` - messages from a specific sender
- `top_msg_id(id)` - search within a specific thread or topic
- `.fetch()` - execute

**Global search** via `GlobalSearchBuilder` (`.search_global_builder(query)`):

- `folder_id`, `broadcasts_only`, `groups_only`, `users_only`
- `filter(MessagesFilter)`, `min_date`, `max_date`
- `offset_rate`, `offset_id`, `limit`
- `.fetch()` - execute

Also: `search_messages(peer, query)` and `search_global(query)` for quick one-liners, `search_contacts(query)` for contacts.

---

## Participants and Rights

- `get_participants(peer, filter)` - fetch members with a filter type
- `iter_participants(peer, filter)` - paginated participant iterator
- `kick_participant(peer, user)` - remove from group
- `ban_participant(peer, user, rights, until)` - ban with optional expiry
- `promote_participant(peer, user, rights)` - grant admin rights
- `set_banned_rights(peer, user, BannedRightsBuilder)` - fine-grained restriction
- `set_admin_rights(peer, user, AdminRightsBuilder)` - fine-grained admin grant
- `get_permissions(peer, user)` - get current permissions for a user
- `get_chat_administrators(peer)` - list all admins
- `get_admins_with_invites(peer)` - list admins who created invite links
- `search_peer(peer, query)` - search members by name
- `transfer_chat_ownership(peer, user, password)` - transfer creator status

**`BannedRightsBuilder`** toggles:

`view_messages`, `send_messages`, `send_media`, `send_stickers`, `send_gifs`, `send_games`, `send_inline`, `embed_links`, `send_polls`, `change_info`, `invite_users`, `pin_messages`, `until_date(ts)`, `full_ban()`

**`AdminRightsBuilder`** toggles:

`change_info`, `post_messages`, `edit_messages`, `delete_messages`, `ban_users`, `invite_users`, `pin_messages`, `add_admins`, `anonymous`, `manage_call`, `manage_topics`, `rank(str)`, `full_admin()`

**`ParticipantPermissions`**: `is_creator`, `is_admin`, `is_banned`, `is_member`

---

## Profile Photos

- `set_profile_photo(media)` - upload and set profile photo
- `delete_profile_photos(ids)` - delete one or more profile photos
- `get_profile_photos(user)` - list profile photos (paginated)
- `iter_profile_photos(user)` - streaming `ProfilePhotosIter` with `.total_count()` and `.collect()`

---

## Profile and Account

- `update_profile(first_name, last_name, about)` - update name and bio
- `update_username(username)` - change username
- `update_status(offline)` - set online or offline
- `set_emoji_status(document_id, until)` - set emoji status with optional expiry

---

## Chats and Channels

- `create_group(title, users)` - create a basic group
- `create_channel(title, about, megagroup)` - create broadcast channel or supergroup
- `delete_channel(peer)` - delete a channel or supergroup
- `delete_chat(chat_id)` - delete a basic group
- `leave_chat(peer)` - leave any chat
- `edit_chat_title(peer, title)` - rename
- `edit_chat_about(peer, about)` - update description
- `edit_chat_photo(peer, media)` - set chat photo
- `edit_chat_default_banned_rights(peer, BannedRightsBuilder)` - default permissions for new members
- `migrate_chat(chat_id)` - upgrade group to supergroup
- `invite_users(peer, users)` - add users to a chat
- `set_history_ttl(peer, period)` - set auto-delete timer
- `toggle_no_forwards(peer, enabled)` - prevent forwarding and saving
- `set_chat_theme(peer, emoticon)` - set chat theme
- `set_chat_reactions(peer, reactions)` - configure allowed reactions
- `get_send_as_peers(peer)` - list peers available for anonymous posting
- `set_default_send_as(peer, send_as)` - set default send-as identity
- `toggle_forum(peer, enabled)` - enable or disable forum mode
- `get_linked_channel(peer)` - get linked discussion group or broadcast
- `get_admin_log(peer, ...)` - fetch admin action log with filters
- `get_common_chats(user, limit)` - groups shared between you and another user
- `send_chat_action(peer, action)` - send typing/uploading/recording indicator
- `join_chat(peer)` - join by username or public link
- `accept_invite_link(link)` - join via invite link
- `parse_invite_hash(link)` - extract invite hash from a link string

---

## Invite Links

- `export_invite_link(peer)` - get or create the primary link
- `revoke_invite_link(peer, link)` - revoke a link
- `edit_invite_link(peer, link, ...)` - update expiry, member limit, or title
- `get_invite_links(peer, admin)` - list all links for an admin
- `delete_invite_link(peer, link)` - delete a specific link
- `delete_revoked_invite_links(peer, admin)` - bulk delete revoked links
- `approve_join_request(peer, user)` - approve a single request
- `reject_join_request(peer, user)` - reject a single request
- `approve_all_join_requests(peer, link)` - bulk approve all pending
- `reject_all_join_requests(peer, link)` - bulk reject all pending
- `get_invite_link_members(peer, link)` - users who joined via a specific link

---

## Contacts

- `get_contacts()` - fetch full contact list
- `add_contact(user, first_name, last_name, phone, add_phone_privacy_exception)`
- `delete_contacts(user_ids)` - remove contacts
- `import_contacts(Vec<InputContact>)` - bulk import
- `block_user(peer)` - block
- `unblock_user(peer)` - unblock
- `get_blocked_users(offset, limit)` - paginated blocked users list
- `search_contacts(query)` - search contacts by name

---

## Forum Topics

- `get_forum_topics(peer)` - list all topics
- `get_forum_topics_by_id(peer, ids)` - fetch specific topics by ID
- `create_forum_topic(peer, title, icon_emoji, icon_color)`
- `edit_forum_topic(peer, topic_id, ...)` - rename or change icon
- `delete_forum_topic_history(peer, top_msg_id)` - delete topic messages
- `toggle_forum(peer, enabled)` - enable/disable forum mode on a supergroup

---

## Polls

- `send_poll(peer, question, answers, ...)` with options:
  - Regular poll or quiz (with correct answer and explanation text)
  - Multiple answers
  - Close date
- `send_vote(peer, msg_id, options)` - vote in a poll
- `get_poll_results(peer, msg_id)` - current results
- `get_poll_votes(peer, msg_id, option, limit)` - voters per option

---

## Reactions

- `InputReactions::emoticon(str)` - standard emoji
- `InputReactions::custom_emoji(document_id)` - custom emoji
- `InputReactions::remove()` - remove reaction
- `.big()` - animated big reaction
- `.add_to_recent()` - add to recent list
- `msg.react(InputReactions)` / `msg.react_with(client, InputReactions)`
- `get_message_reactions(peer, ids)` - reaction counts per message
- `get_reaction_list(peer, msg_id, reaction, limit)` - users who reacted
- `send_paid_reaction(peer, msg_id, count)` - send paid star reactions
- `read_reactions(peer)` - mark reactions as read
- `clear_recent_reactions()` - clear recent reactions

---

## Stickers and Custom Emoji

- `get_sticker_set(InputStickerSet)` - by short name or ID
- `install_sticker_set(set, archived)`
- `uninstall_sticker_set(set)`
- `get_all_stickers(hash)` - all installed sets
- `get_custom_emoji_documents(ids)` - fetch custom emoji documents

---

## Games

- `set_game_score(peer, user, score, force, disable_edit)`
- `get_game_high_scores(peer, msg_id, user)`

---

## Payments and Invoices

- `send_invoice(peer, title, description, payload, provider, prices, ...)` - send an invoice message
- `answer_shipping_query(query_id, ok, shipping_options, error)`
- `answer_precheckout_query(query_id, error)` - confirm or reject checkout
- `ShippingQuery` and `PreCheckoutQuery` update types with full accessors

---

## Inline Mode

- `answer_inline_query(query_id, results, cache_time, personal, next_offset, switch_pm, ...)` - answer inline query
- `iter_inline_queries()` - stream `InlineQuery` updates as `InlineQueryIter`
- `inline_query(bot, peer, query)` - send inline query programmatically and iterate results
- `edit_inline_message(id, InputMessage)` - edit an inline result message
- `InlineQuery` accessors: `id`, `query`, `offset`, `peer_type`, `geo`
- `InlineResult` accessors: `id`, `title`, `description`, `.send(peer)`

---

## Callback Queries

- `answer_callback_query(id, ...)` - acknowledge a button press
- `Answer` builder on `CallbackQuery`: `.text(str)`, `.alert(str)`, `.url(str)`, `.cache_time(Duration)`, `.send(client)`
- `answer_flat(client, text)` - quick toast notification
- `answer_alert(client, text)` - quick alert popup
- `CallbackQuery` accessors: `data`, `msg_id`, `chat_instance`, `game_short_name`

---

## Keyboards and Reply Markup

**`Button`** factory methods:

- `callback(text, data)` - inline button with callback data
- `url(text, url)` - inline URL button
- `url_auth(text, url, ...)` - login URL button
- `switch_inline(text, query)` - switch to inline in current chat
- `switch_elsewhere(text, query)` - switch to inline in another chat
- `webview(text, url)` - full web app button
- `simple_webview(text, url)` - simple web app button
- `request_phone(text)` - request user's contact
- `request_geo(text)` - request user's location
- `request_poll(text)` - request poll creation
- `request_quiz(text)` - request quiz creation
- `game(text)` - open game
- `buy(text)` - payment button
- `copy_text(text, copy_text)` - copies text to clipboard

**`InlineKeyboard`**: `.row(buttons)`, `.into_markup()`

**`ReplyKeyboard`**: `.row(buttons)`, `.resize()`, `.single_use()`, `.selective()`, `.into_markup()`

---

## TypingGuard

RAII typing indicator. Cancels automatically on drop.

- `client.typing(peer)` - general typing action
- `client.typing_in_topic(peer, topic_id)` - typing in a forum topic
- `client.uploading_document(peer)` - document upload indicator
- `client.recording_video(peer)` - video recording indicator
- `TypingGuard::start(client, peer, action)` - any raw `SendMessageAction`
- `TypingGuard::start_ex(client, peer, topic_id, action)` - with topic ID
- `guard.cancel()` - explicit cancel

---

## Dispatcher and Routing

- `Dispatcher::new()` - top-level dispatcher
- `Router::new()` - sub-router for handler grouping
- `dp.include(router)` - mount a sub-router
- `dp.scope(filter)` - apply a filter guard to all handlers in a router
- `dp.on_message(filter, handler)` - register a new message handler
- `dp.on_edit(filter, handler)` - register an edited message handler
- `dp.on_message_fsm(filter, state, handler)` - FSM-guarded message handler
- `dp.on_edit_fsm(filter, state, handler)` - FSM-guarded edit handler
- `dp.middleware(mw)` - register a middleware
- `dp.with_state_storage(Arc<dyn StateStorage>)` - set FSM backend
- `dp.with_key_strategy(StateKeyStrategy)` - set state key scope
- `dp.dispatch(update)` - dispatch one update through handlers
- `dispatch!` macro - ergonomic match-style dispatch without a Dispatcher struct

---

## Filters

All return `BoxFilter`. Compose with `&`, `|`, `!`:

`all`, `none`, `private`, `group`, `channel`, `text`, `media`, `photo`, `document`, `forwarded`, `reply`, `album`, `any_command`, `command(name)`, `text_contains(needle)`, `text_starts_with(prefix)`, `from_user(id)`, `in_chat(id)`, `custom(fn)`

---

## Middleware

- `Middleware` trait: implement `call(update, next)` for custom logic
- `TracingMiddleware` - logs each update at debug level
- `RateLimitMiddleware::new(max_calls, window)` - per-user sliding window rate limit; `tracked_users()` reports active users
- `PanicRecoveryMiddleware` - catches handler panics, logs them, continues dispatch
- `Next::run(update)` - pass update to the next handler in chain
- `DispatchError` - structured error with `.msg(str)` and `.wrap(err)` constructors

---

## FSM

- `on_message_fsm(filter, State::Variant, handler)` - handler fires only when user is in that state
- `on_edit_fsm(filter, State::Variant, handler)` - same for edited messages
- `StateKeyStrategy`: `PerUser`, `PerChat`, `PerUserInChat`
- `MemoryStorage` - in-process HashMap-backed storage
- Custom storage: implement `StateStorage` trait
- `state.transition(S)` - move to next state
- `state.clear_state()` - reset to no state
- `state.set_data(field, value)` - attach serde-serializable data
- `state.get_data::<T>(field)` - retrieve typed data
- `state.get_all_data()` - all data as `HashMap<String, Value>`
- `state.clear_data()` - delete data, keep state
- `state.clear_all()` - delete state and all data
- `state.key()` - inspect active state key
- `#[derive(FsmState)]` on enums: generates `as_key() -> String` and `from_key(&str) -> Option<Self>` (unit variants only, from `ferogram-derive`)

---

## Conversations

- `Conversation::new(client, peer)` - open a sequential request-response session
- `conv.ask(text)` - send a message and wait for the next reply
- `conv.respond(InputMessage)` - send without waiting
- `conv.get_response()` - wait for the next incoming message
- `conv.wait_click(deadline)` - wait for a callback query click
- `conv.wait_read(deadline)` - wait until messages are read
- `conv.ask_and_wait(text, deadline)` - send and wait with a deadline
- `conv.peer()` - the conversation's peer
- `conv.drain_buffered()` - collect updates buffered during the conversation

---

## Text Formatting

- `InputMessage::text(str)` - plain text
- `InputMessage::markdown(str)` - parse Markdown entities at send time
- `InputMessage::html(str)` - parse HTML entities at send time
- `parse_html(str) -> (String, Vec<MessageEntity>)` - built-in HTML parser (`html` feature)
- `generate_html(text, entities) -> String` - render entities back to HTML
- `parse_markdown(str) -> (String, Vec<MessageEntity>)` - Markdown parser
- `html5ever`-based HTML parser as a spec-compliant alternative (`html5ever` feature)
- `msg.markdown_text()` - re-renders message text with Markdown formatting applied
- `msg.html_text()` - re-renders message text with HTML formatting applied

---

## Web Pages

- `get_web_page_preview(url)` - fetch Telegram web page preview for a URL

---

## Discussions

- `get_replies(peer, msg_id, ...)` - fetch comments under a channel post
- `get_discussion_message(peer, msg_id)` - get the linked discussion message
- `read_discussion(peer, msg_id, read_max_id)` - mark discussion as read

---

## Translation and Transcription

- `translate_messages(peer, ids, to_lang)` - translate via Telegram's built-in translator
- `transcribe_audio(peer, msg_id)` - voice-to-text transcription
- `toggle_peer_translations(peer, disabled)` - enable/disable translation bar for a chat

---

## Privacy

- `get_privacy(key)` - current rule for a key
- `set_privacy(key, rules)` - set rule for a key

Keys: last seen, profile photo, phone number, bio, birthday, forwards, calls, voice messages, group add, gifts.

Audiences: everybody, contacts, close friends, nobody, plus explicit allow/disallow lists.

- `get_notify_settings(peer)` - fetch notification settings for a peer
- `update_notify_settings(peer, settings)` - mute, sound, alerts, etc.

---

## Statistics

- `get_broadcast_stats(peer)` - channel stats: followers, views, shares, message activity
- `get_megagroup_stats(peer)` - supergroup stats: growth, languages, member activity

---

## Bot Configuration

- `set_bot_commands(scope, lang_code, commands)` - set command list scoped by chat type and language
- `delete_bot_commands(scope, lang_code)` - remove commands for a scope
- `set_bot_info(bot, lang_code, name, about, description)` - set bot metadata
- `get_bot_info(bot, lang_code)` - fetch bot name, about, description
- `start_bot(bot, peer, start_param)` - programmatically start a bot with a deep link parameter

---

## Dice and Animated Emoji

- `send_dice(peer, emoticon)` - send animated dice, dart, basketball, football, slots, or bowling

---

## Raw API

- `client.invoke(&req)` - call any TL function against the current DC
- `client.invoke_on_dc(dc_id, &req)` - call any TL function against a specific DC
- `rpc_call_raw_pub` / `rpc_on_dc_raw_pub` / `rpc_transfer_on_dc_pub` - low-level RPC helpers
- `cache_users_slice_pub` / `cache_chats_slice_pub` - manually seed the access-hash cache
- `sync_update_state()` - force sync of PTS/QTS state
- Full Layer 224 type coverage via `ferogram::tl`: `tl::types`, `tl::enums`, `tl::functions`

---

## TL Layer Crates

- `ferogram-tl-types` - 2,329 TL definitions generated at build time from Layer 224; binary TL serialization and deserialization on all types
- `ferogram-tl-gen` - build-time code generator from a TL AST to Rust source
- `ferogram-tl-parser` - streaming parser for `.tl` schema files; produces a typed `Definition` AST; `TlIterator` for low-memory streaming; parse errors include the failing line

---

## Cryptography (`ferogram-crypto`)

All implemented from scratch, no external crypto service:

- AES-IGE encrypt/decrypt - MTProto message encryption
- AES-CTR - CDN file decryption
- RSA-OAEP - DH handshake
- SHA-1 and SHA-256
- Diffie-Hellman key generation and server parameter verification
- `ObfCipher` - obfuscated transport key derivation
- `derive_keys` - auth key derivation from DH output
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
- 18 unit tests for PTS gap recovery logic in `pts_tests.rs`
- `cargo test --workspace` runs all crates
- `cargo test --workspace --all-features` covers optional backends
- `Send + Sync` on all public types
- Works on Linux, macOS, Windows
- Works in Termux (Android) with the native Rust toolchain
- Async throughout, built on Tokio
- No blocking mutex in async hot paths
