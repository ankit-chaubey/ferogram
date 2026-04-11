# Client Methods: Full Reference

All methods on `Client`. Every method is `async` and returns `Result<T, InvocationError>` unless noted.

---

## Connection & Session

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">Client::connect(config: Config) → Result&lt;(Client, ShutdownToken), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Opens a TCP connection to Telegram, performs the full 3-step DH key exchange, and loads any existing session. Returns both the client handle and a <code>ShutdownToken</code> for graceful shutdown.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Client::with_string_session(session: &str, api_id: i32, api_hash: &str) → Result&lt;(Client, ShutdownToken), InvocationError&gt; <span class="api-badge-new">New 0.1.1</span></span>
</div>
<div class="api-card-body">
Convenience constructor that connects using a <code>StringSessionBackend</code>. Pass the string exported by <code>export_session_string()</code>.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.is_authorized() → Result&lt;bool, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Returns <code>true</code> if the session has a logged-in user or bot. Use this to skip the login flow on subsequent runs.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.save_session() → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Writes the current session (auth key + DC info + peer cache) to the backend. Call after a successful login.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.export_session_string() → Result&lt;String, InvocationError&gt; <span class="api-badge-new">New 0.1.1</span></span>
</div>
<div class="api-card-body">
Serialises the current session to a portable base64 string. Store it in an env var, DB column, or CI secret. Restore with <code>Client::with_string_session()</code> or <code>StringSessionBackend</code>.
<pre><code>let s = client.export_session_string().await?;
std::env::set_var("TG_SESSION", &s);</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.sign_out() → Result&lt;bool, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Revokes the auth key on Telegram's servers and deletes the local session. The bool indicates whether teardown was confirmed server-side.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.disconnect()</span>
</div>
<div class="api-card-body">
Immediately closes the TCP connection and stops the reader task without waiting for pending RPCs to drain. For graceful shutdown that waits for pending calls, use <code>ShutdownToken::cancel()</code> instead.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.sync_update_state() <span class="api-badge-new">New 0.1.1</span></span>
</div>
<div class="api-card-body">
Forces an immediate <code>updates.getState</code> round-trip and reconciles local pts/seq/qts counters. Useful after a long disconnect or when you suspect a gap but don't want to wait for the gap-detection timer.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.signal_network_restored()</span>
</div>
<div class="api-card-body">
Signals to the reconnect logic that the network is available. Skips the exponential backoff and triggers an immediate reconnect attempt. Call from Android <code>ConnectivityManager</code> or iOS <code>NWPathMonitor</code> callbacks.
</div>
</div>

---

## Authentication

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.request_login_code(phone: &str) → Result&lt;LoginToken, InvocationError&gt;</span>
</div>
<div class="api-card-body">Sends a verification code to <code>phone</code> via SMS or Telegram app. Returns a <code>LoginToken</code> to pass to <code>sign_in</code>. Phone must be in E.164 format: <code>"+12345678900"</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.sign_in(token: &LoginToken, code: &str) → Result&lt;String, SignInError&gt;</span>
</div>
<div class="api-card-body">
Submits the verification code. Returns the user's full name on success, or <code>SignInError::PasswordRequired(PasswordToken)</code> when 2FA is enabled. The <code>PasswordToken</code> carries the hint set by the user.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.check_password(token: PasswordToken, password: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Completes the SRP 2FA verification. The password is never transmitted in plain text: only a zero-knowledge cryptographic proof is sent.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.bot_sign_in(token: &str) → Result&lt;String, InvocationError&gt;</span>
</div>
<div class="api-card-body">Logs in using a bot token from @BotFather. Returns the bot's username on success.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_me() → Result&lt;tl::types::User, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetches the full <code>User</code> object for the logged-in account. Contains <code>id</code>, <code>username</code>, <code>first_name</code>, <code>last_name</code>, <code>phone</code>, <code>bot</code> flag, <code>verified</code> flag, and more.</div>
</div>

---

## Updates

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.stream_updates() → UpdateStream</span>
</div>
<div class="api-card-body">
Returns an <code>UpdateStream</code>: an async iterator that yields typed <code>Update</code> values. Call <code>.next().await</code> in a loop to process events. The stream runs until the connection is closed.
<pre><code>let mut updates = client.stream_updates();
while let Some(update) = updates.next().await {
    match update {
        Update::NewMessage(msg) => { /* … */ }
        _ => {}
    }
}</code></pre>
</div>
</div>

---

## Messaging

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_message(peer: &str, text: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send a plain-text message. <code>peer</code> can be <code>"me"</code>, <code>"@username"</code>, or a numeric ID string. For rich formatting, use <code>send_message_to_peer_ex</code>.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_to_self(text: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Sends a message to your own Saved Messages. Shorthand for <code>send_message("me", text)</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_message_to_peer(peer: Peer, text: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Send a plain text message to a resolved <code>tl::enums::Peer</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_message_to_peer_ex(peer: Peer, msg: &InputMessage) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Full-featured send with the <a href="./input-message.md"><code>InputMessage</code></a> builder: supports markdown entities, reply-to, inline keyboard, scheduled date, silent flag, and more.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_message(peer: Peer, message_id: i32, new_text: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Edit the text of a previously sent message. Only works on messages sent by the logged-in account (or bot).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_inline_message(inline_msg_id: tl::enums::InputBotInlineMessageId, text: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Edit the text of a message that was sent via inline mode. The <code>inline_msg_id</code> is provided in <code>Update::InlineSend</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.forward_messages(from_peer: Peer, to_peer: Peer, ids: Vec&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Forward one or more messages from <code>from_peer</code> into <code>to_peer</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_messages(ids: Vec&lt;i32&gt;, revoke: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body"><code>revoke: true</code> deletes for everyone; <code>false</code> deletes only for the current account.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_messages_by_id(peer: Peer, ids: &[i32]) → Result&lt;Vec&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch specific messages by their IDs from a peer. Returns messages in the same order as the input IDs.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.pin_message(peer: Peer, message_id: i32, silent: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Pin a message. <code>silent: true</code> pins without notifying members.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unpin_message(peer: Peer, message_id: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Unpin a specific message.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unpin_all_messages(peer: Peer) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Unpin all pinned messages in a chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_pinned_message(peer: Peer) → Result&lt;Option&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the currently pinned message, or <code>None</code> if nothing is pinned.</div>
</div>

---

## Reactions

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_reaction(peer: Peer, msg_id: i32, reaction: Reaction) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send a reaction to a message. Build reactions using the <code>Reaction</code> helper:
<pre><code>use ferogram::reactions::InputReactions;

client.send_reaction(peer, msg_id, InputReactions::emoticon("👍")).await?;
client.send_reaction(peer, msg_id, InputReactions::remove()).await?; // remove all
client.send_reaction(peer, msg_id, InputReactions::emoticon("🔥").big()).await?;</code></pre>
See <a href="../messaging/reactions.md">Reactions</a> for the full guide.
</div>
</div>

---

## Sending chat actions

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_chat_action(peer: Peer, action: SendMessageAction, top_msg_id: Option&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send a one-shot typing / uploading / recording indicator. Expires after ~5 seconds. Use <a href="./typing-guard.md"><code>TypingGuard</code></a> to keep it alive for longer operations. <code>top_msg_id</code> restricts the indicator to a forum topic.
</div>
</div>

---

## Search

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.search(peer: impl Into&lt;PeerRef&gt;, query: &str) → SearchBuilder</span>
</div>
<div class="api-card-body">Returns a <a href="./search.md"><code>SearchBuilder</code></a> for per-peer message search with date filters, sender filter, media type filter, and pagination.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.search_global_builder(query: &str) → GlobalSearchBuilder</span>
</div>
<div class="api-card-body">Returns a <a href="./search.md"><code>GlobalSearchBuilder</code></a> for searching across all dialogs simultaneously.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.search_messages(peer: Peer, query: &str, limit: i32) → Result&lt;Vec&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Simple one-shot search within a peer. For advanced options use <code>client.search()</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.search_global(query: &str, limit: i32) → Result&lt;Vec&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Simple one-shot global search. For advanced options use <code>client.search_global_builder()</code>.</div>
</div>

---

## Dialogs & History

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_dialogs(limit: i32) → Result&lt;Vec&lt;Dialog&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the most recent <code>limit</code> dialogs. Each <code>Dialog</code> has <code>title()</code>, <code>peer()</code>, <code>unread_count()</code>, and <code>top_message()</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.iter_dialogs() → DialogIter</span>
</div>
<div class="api-card-body">Lazy iterator that pages through <em>all</em> dialogs automatically. Call <code>iter.next(&client).await?</code>. <code>iter.total()</code> returns the server-reported count after the first page.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">client.iter_messages(peer: impl Into&lt;PeerRef&gt;) → MessageIter</span>
</div>
<div class="api-card-body">Lazy iterator over the full message history of a peer, newest first. Call <code>iter.next(&client).await?</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_messages(peer: Peer, limit: i32, offset_id: i32) → Result&lt;Vec&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch a page of messages. Pass the lowest message ID from the previous page as <code>offset_id</code> to paginate.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.mark_as_read(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Mark all messages in a dialog as read.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.clear_mentions(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Clear unread @mention badges in a chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_dialog(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Delete a dialog from the account's dialog list (does not delete messages for others).</div>
</div>

---

## Scheduled messages

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_scheduled_messages(peer: Peer) → Result&lt;Vec&lt;tl::enums::Message&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch all messages scheduled to be sent in a chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_scheduled_messages(peer: Peer, ids: Vec&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Cancel and delete scheduled messages by ID.</div>
</div>

---

## Participants & Admin

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_participants(peer: Peer, limit: i32) → Result&lt;Vec&lt;Participant&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch members of a chat or channel. Pass <code>limit = 0</code> for the default server maximum per page. Use <code>iter_participants</code> to lazily page all members.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.iter_participants(peer: Peer) → ParticipantIter</span>
</div>
<div class="api-card-body">Lazy async iterator that pages through all members, including beyond the 200-member limit. Fixed in v0.1.1 to paginate correctly for large channels.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_admin_rights(peer: Peer, user_id: i64, rights: AdminRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Promote a user to admin with specified rights. See <a href="./admin-rights.md">Admin & Ban Rights</a>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_banned_rights(peer: Peer, user_id: i64, rights: BanRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Restrict or ban a user. Pass <code>BanRightsBuilder::full_ban()</code> to fully ban. See <a href="./admin-rights.md">Admin & Ban Rights</a>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_profile_photos(peer: Peer, limit: i32) → Result&lt;Vec&lt;tl::enums::Photo&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch a user's profile photo list.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_permissions(peer: Peer, user_id: i64) → Result&lt;Participant, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the effective permissions of a user in a chat. Check <code>.is_admin()</code>, <code>.is_banned()</code>, etc. on the returned <code>Participant</code>.</div>
</div>

---

## Media

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_file(path: &str) → Result&lt;UploadedFile, InvocationError&gt;</span>
</div>
<div class="api-card-body">Upload a file from a local path. Returns an <code>UploadedFile</code> with <code>.as_photo_media()</code> and <code>.as_document_media()</code> methods.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_file(peer: Peer, media: InputMedia, caption: Option&lt;&str&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Send an uploaded file as a photo or document with an optional caption.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_album(peer: Peer, media: Vec&lt;InputMedia&gt;, caption: Option&lt;&str&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Send multiple media items as a grouped album (2–10 items).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.download_media_to_file(location: impl Downloadable, path: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Download a media attachment and write it directly to a file path.</div>
</div>

---

## Callbacks & Inline

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.answer_callback_query(query_id: i64, text: Option&lt;&str&gt;, alert: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Acknowledge an inline button press. <code>text</code> shows a toast (or alert if <code>alert=true</code>). Must be called within 60 seconds of the button press.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.answer_inline_query(query_id: i64, results: Vec&lt;InputBotInlineResult&gt;, cache_time: i32, is_personal: bool, next_offset: Option&lt;&str&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Respond to an inline query with a list of results. <code>cache_time</code> in seconds. Empty result list now handled correctly (fixed in v0.1.1).</div>
</div>

---

## Peer resolution

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.resolve_peer(peer: &str) → Result&lt;tl::enums::Peer, InvocationError&gt;</span>
</div>
<div class="api-card-body">Resolve a string (<code>"@username"</code>, <code>"+phone"</code>, <code>"me"</code>, numeric ID) to a <code>Peer</code> with cached access hash.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.resolve_username(username: &str) → Result&lt;tl::enums::Peer, InvocationError&gt;</span>
</div>
<div class="api-card-body">Resolve a bare username (without @) to a <code>Peer</code>.</div>
</div>

---

## Raw API

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.invoke&lt;R: RemoteCall&gt;(req: &R) → Result&lt;R::Return, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Call any Layer 224 API method directly. See <a href="../advanced/raw-api.md">Raw API Access</a> for the full guide.
<pre><code>use ferogram_tl_types::functions;
let state = client.invoke(&functions::updates::GetState {}).await?;</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.invoke_on_dc&lt;R: RemoteCall&gt;(req: &R, dc_id: i32) → Result&lt;R::Return, InvocationError&gt;</span>
</div>
<div class="api-card-body">Send a request to a specific Telegram data centre. Used for file downloads from CDN DCs.</div>
</div>

---

## Chat management

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.join_chat(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Join a group or channel by peer reference.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.accept_invite_link(link: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Accept a <code>t.me/+hash</code> or <code>t.me/joinchat/hash</code> invite link.</div>
</div>
