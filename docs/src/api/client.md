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
<span class="api-card-sig">Config::with_string_session(s: impl Into&lt;String&gt;) → Config <span class="api-badge-new">New 0.2.0</span></span>
</div>
<div class="api-card-body">
Convenience constructor that builds a <code>Config</code> pre-wired with a <code>StringSessionBackend</code>. Pass the string exported by <code>export_session_string()</code>, or an empty string to start a fresh session. All other <code>Config</code> fields default to their standard values.

<pre><code>let cfg = Config {
    api_id:   12345,
    api_hash: "abc".into(),
    catch_up: true,
    ..Config::with_string_session(std::env::var("SESSION").unwrap_or_default())
};
let (client, _token) = Client::connect(cfg).await?;</code></pre>
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
<span class="api-card-sig">client.export_session_string() → Result&lt;String, InvocationError&gt; <span class="api-badge-new">New 0.2.0</span></span>
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
<span class="api-card-sig">client.sync_update_state() <span class="api-badge-new">New 0.2.0</span></span>
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
<span class="api-card-sig">client.send_message(peer: &str, text: &str) → Result&lt;IncomingMessage, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send a plain-text message. <code>peer</code> can be <code>"me"</code>, <code>"@username"</code>, or a numeric ID string. Pass an <code>InputMessage</code> for rich formatting, keyboard, or media.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_to_self(text: &str) → Result&lt;IncomingMessage, InvocationError&gt;</span>
</div>
<div class="api-card-body">Sends a message to your own Saved Messages. Shorthand for <code>send_message("me", text)</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_message(peer: Peer, text: &str) → Result&lt;IncomingMessage, InvocationError&gt;</span>
</div>
<div class="api-card-body">Send a plain text message to a resolved <code>tl::enums::Peer</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_message(peer: impl Into&lt;PeerRef&gt;, msg: impl Into&lt;InputMessage&gt;) → Result&lt;IncomingMessage, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Full-featured send with the <a href="./input-message.md"><code>InputMessage</code></a> builder: supports markdown entities, reply-to, inline keyboard, scheduled date, silent flag, and more. A bare <code>&str</code> or <code>String</code> is accepted as a shorthand for plain text.
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
<span class="api-card-sig">client.get_messages_by_id(peer: Peer, ids: &[i32]) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
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
<span class="api-card-sig">client.get_pinned_message(peer: Peer) → Result&lt;Option&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the currently pinned message, or <code>None</code> if nothing is pinned.</div>
</div>

---

## Reactions

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_reaction(peer: Peer, msg_id: i32, reaction: impl Into&lt;InputReactions&gt;) → Result&lt;(), InvocationError&gt;</span>
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

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_message_reactions(peer: impl Into&lt;PeerRef&gt;, msg_ids: Vec&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Trigger a server push of the current reaction counters for the given message IDs. The server responds with <code>updateMessageReactions</code> updates in the stream.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_reaction_list(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, reaction: Option&lt;tl::enums::Reaction&gt;, limit: i32, offset: Option&lt;String&gt;) → Result&lt;tl::types::messages::MessageReactionsList, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the list of users who reacted to a message. Pass <code>reaction = None</code> for all reactions. <code>limit</code> max 100; use <code>offset</code> from the previous response to paginate.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_paid_reaction(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, count: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Send a paid (Stars) reaction to a message. <code>count</code> is the number of Stars to spend.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.read_reactions(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Mark all unread reactions in a chat as seen (clears reaction badges).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.clear_recent_reactions() → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Clear the recent reactions list shown in the reaction picker.</div>
</div>

---

## Sending chat actions

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_chat_action(peer: Peer, action: SendMessageAction) → Result&lt;(), InvocationError&gt;</span>
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
<span class="api-card-sig">client.search_messages(peer: Peer, query: &str, limit: i32) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Simple one-shot search within a peer. For advanced options use <code>client.search()</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.search_global(query: &str, limit: i32) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
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
<span class="api-card-sig">client.get_messages(peer: Peer, limit: i32, offset_id: i32) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
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

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_chat_history(peer: impl Into&lt;PeerRef&gt;, max_id: i32, revoke: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Delete message history up to <code>max_id</code>. Pass <code>max_id = 0</code> to delete everything. <code>revoke = true</code> also removes messages for all other participants (requires admin rights in channels).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.pin_dialog(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Pin a dialog to the top of the dialog list.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unpin_dialog(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Unpin a previously pinned dialog.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_pinned_dialogs(folder_id: i32) → Result&lt;Vec&lt;tl::enums::Dialog&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch all pinned dialogs in a folder. Use <code>folder_id = 0</code> for the main list, <code>1</code> for the archive.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.mark_dialog_unread(peer: impl Into&lt;PeerRef&gt;, unread: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Manually mark a dialog as unread (<code>true</code>) or read (<code>false</code>). This sets the unread dot without actually having new messages.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.count_channels() → Result&lt;usize, InvocationError&gt;</span>
</div>
<div class="api-card-body">Count how many channel dialogs the logged-in account is currently in. Iterates all dialogs internally.</div>
</div>

---

## Scheduled messages

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_scheduled_messages(peer: impl Into&lt;PeerRef&gt;) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch all messages currently scheduled in a chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_scheduled_messages(peer: impl Into&lt;PeerRef&gt;, ids: Vec&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Cancel and delete scheduled messages by their scheduled message IDs.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_scheduled_now(peer: impl Into&lt;PeerRef&gt;, ids: Vec&lt;i32&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Immediately deliver one or more scheduled messages. <code>ids</code> are the scheduled message IDs returned by <code>get_scheduled_messages</code>.</div>
</div>

---

## Participants & Admin

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_participants(peer: impl Into&lt;PeerRef&gt;, limit: i32) → Result&lt;Vec&lt;Participant&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch members of a chat or channel. Pass <code>limit = 0</code> for the default server maximum per page. Use <code>iter_participants</code> to lazily page all members.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.iter_participants(peer: impl Into&lt;PeerRef&gt;, filter: Option&lt;tl::enums::ChannelParticipantsFilter&gt;, limit: i32) → Result&lt;Vec&lt;Participant&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch all members of a channel or supergroup, optionally filtered and limited. Pass <code>filter = None</code> and <code>limit = 0</code> to retrieve all members up to the server default. For basic groups use <code>get_participants</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.kick_participant(chat_id: i64, user_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Remove a user from a basic group by chat ID. For channels and supergroups use <code>ban_participant</code> instead (with <code>until_date = 0</code> for a permanent ban, or unban immediately after).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.ban_participant(channel: impl Into&lt;PeerRef&gt;, user_id: i64, until_date: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Ban a user from a channel or supergroup. <code>until_date</code> is a Unix timestamp; pass <code>0</code> for a permanent ban. To unban, pass a past timestamp or call again with <code>0</code> and then immediately unban via <code>set_banned_rights</code> with an empty builder.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.promote_participant(channel: impl Into&lt;PeerRef&gt;, user_id: i64, promote: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Quick-promote (<code>true</code>) or demote (<code>false</code>) a user in a channel or supergroup. Promotion grants all standard admin rights except <code>add_admins</code>. For fine-grained control use <code>set_admin_rights</code> with <code>AdminRightsBuilder</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_admin_rights(peer: impl Into&lt;PeerRef&gt;, user_id: i64, rights: AdminRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Promote a user to admin with specified rights. See <a href="./admin-rights.md">Admin & Ban Rights</a>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_banned_rights(peer: impl Into&lt;PeerRef&gt;, user_id: i64, rights: BannedRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Restrict or ban a user. Pass <code>BannedRightsBuilder::full_ban()</code> to fully ban. See <a href="./admin-rights.md">Admin & Ban Rights</a>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_profile_photos(peer: impl Into&lt;PeerRef&gt;, limit: i32) → Result&lt;Vec&lt;tl::enums::Photo&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch a page of a user's profile photos.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.iter_profile_photos(peer: impl Into&lt;PeerRef&gt;, chunk_size: i32) → Result&lt;ProfilePhotoIter, InvocationError&gt;</span>
</div>
<div class="api-card-body">Lazy iterator over all profile photos of a user, fetched in pages of <code>chunk_size</code>. Pass <code>chunk_size = 0</code> for the default (100). Call <code>iter.next().await?</code> to advance. Only works for users.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.search_peer(query: &str) → Result&lt;Vec&lt;tl::enums::Peer&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Search for a peer (user, group, or channel) by name prefix. Searches contacts, dialogs, and globally. Returns combined results.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_permissions(peer: impl Into&lt;PeerRef&gt;, user_id: i64) → Result&lt;ParticipantPermissions, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the effective permissions of a user in a channel or supergroup. See <a href="./participants.md">Participants & Members</a>.</div>
</div>

---

## Media

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_file(data: &[u8], name: &str, mime_type: &str) → Result&lt;UploadedFile, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Upload raw bytes as a file. <code>name</code> is used as the filename; <code>mime_type</code> is auto-detected from the extension if you pass <code>""</code>. Returns an <code>UploadedFile</code> with <code>.as_photo_media()</code> and <code>.as_document_media()</code> methods. For large files, prefer <code>upload_file_concurrent</code>.
<pre><code>let data = std::fs::read("photo.jpg")?;
let uploaded = client.upload_file(&data, "photo.jpg", "image/jpeg").await?;
let media = uploaded.as_photo_media();</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_file(peer: tl::enums::Peer, media: tl::enums::InputMedia, caption: &str) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Send an uploaded file as a photo or document. <code>caption</code> is the message text shown below the media; pass <code>""</code> for no caption (it is <strong>not</strong> <code>Option</code>).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_album(peer: tl::enums::Peer, items: Vec&lt;AlbumItem&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Send 2–10 media items as a grouped album. Each <code>AlbumItem</code> carries its own caption and optional <code>reply_to</code>. See <a href="../messaging/media.md"><code>AlbumItem</code></a> for builder details.
<pre><code>use ferogram::media::AlbumItem;
client.send_album(peer, vec![
    AlbumItem::new(photo1).caption("First"),
    AlbumItem::new(photo2).caption("Second"),
]).await?;</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.download_media_to_file(location: tl::enums::InputFileLocation, path: impl AsRef&lt;Path&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Download a media file and write it to <code>path</code>. The <code>path</code> argument accepts anything that implements <code>AsRef&lt;Path&gt;</code> (e.g. <code>&str</code>, <code>String</code>, <code>PathBuf</code>). Uses DC 0 (auto-routed); for explicit DC routing use <code>download_media_to_file_on_dc</code>.</div>
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
<div class="api-card-body">Respond to an inline query with a list of results. <code>cache_time</code> in seconds. Empty result list now handled correctly (fixed in v0.2.0).</div>
</div>

---

## Peer resolution

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.resolve&lt;P: Into&lt;PeerRef&gt;&gt;(peer: P) → Result&lt;tl::enums::Peer, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Resolve any peer reference to a <code>Peer</code>. Accepts all <code>PeerRef</code> input types:
<ul>
<li><code>&amp;str</code> / <code>String</code> — <code>"@username"</code>, <code>"me"</code>, <code>"self"</code>, numeric string, <code>t.me/</code> URL, invite link, E.164 phone</li>
<li><code>i64</code> / <code>i32</code> — Bot-API encoded numeric ID</li>
<li><code>tl::enums::Peer</code> — returned as-is, zero cost</li>
<li><code>tl::enums::InputPeer</code> — access hash cached, then stripped to <code>Peer</code></li>
</ul>
Resolution is cache-first; an RPC is only made on a cache miss.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.resolve_peer(peer: &str) → Result&lt;tl::enums::Peer, InvocationError&gt;</span>
</div>
<div class="api-card-body">String-only variant of <code>resolve()</code>. Accepts <code>"@username"</code>, <code>"+phone"</code>, <code>"me"</code>, numeric string, <code>t.me/</code> URL, and invite links. Prefer <code>resolve()</code> when the input may not be a string.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.resolve_to_input_peer(peer: &tl::enums::Peer) → Result&lt;tl::enums::InputPeer, InvocationError&gt;</span>
</div>
<div class="api-card-body">Convert a bare <code>Peer</code> to an <code>InputPeer</code> with access hash. Returns an error if the peer has not been seen in a prior API call and the hash is unknown.</div>
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
<span class="api-card-sig">client.invoke_on_dc&lt;R: RemoteCall&gt;(dc_id: i32, req: &R) → Result&lt;R::Return, InvocationError&gt;</span>
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

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.create_group(title: impl Into&lt;String&gt;, user_ids: Vec&lt;i64&gt;) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Create a new basic group with the given title and initial member list. Returns the created <code>Chat</code>. Basic groups support up to 200 members; migrate to supergroup with <code>migrate_chat</code> if you need more.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.create_channel(title: impl Into&lt;String&gt;, about: impl Into&lt;String&gt;, broadcast: bool) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Create a new channel (<code>broadcast = true</code>) or supergroup (<code>broadcast = false</code>). Returns the created <code>Chat</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_channel(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Permanently delete a channel or supergroup. Only the creator can do this. Irreversible.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_chat(chat_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Delete a legacy basic group by its chat ID. Only the creator can do this. Use <code>delete_channel</code> for supergroups and channels.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.leave_chat(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Leave a channel or supergroup. For basic groups, use <code>kick_participant</code> on yourself or <code>delete_dialog</code> to just hide it.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_title(peer: impl Into&lt;PeerRef&gt;, title: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Rename a chat, group, channel, or supergroup. Works for both basic groups and channels/supergroups.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_about(peer: impl Into&lt;PeerRef&gt;, about: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set or update the description/about text for any chat type.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_photo(peer: impl Into&lt;PeerRef&gt;, photo: tl::enums::InputChatPhoto) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Change the profile photo of a chat. Pass <code>tl::enums::InputChatPhoto::Empty</code> to remove the current photo.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_chat_default_banned_rights(peer: impl Into&lt;PeerRef&gt;, build: impl FnOnce(BannedRightsBuilder) → BannedRightsBuilder) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set default permissions for all members of a group or channel via a closure:
<pre><code>// Disable media and polls for everyone
client.edit_chat_default_banned_rights(peer, |b| {
    b.send_media(true).send_polls(true)
}).await?;</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_chat_full(peer: impl Into&lt;PeerRef&gt;) → Result&lt;tl::enums::messages::ChatFull, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the full info object for any chat type. Contains full description, pinned message ID, linked channel, member count, slow mode, and more.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.migrate_chat(chat_id: i64) → Result&lt;tl::enums::Chat, InvocationError&gt;</span>
</div>
<div class="api-card-body">Upgrade a legacy basic group to a supergroup. Returns the new channel peer. The original chat ID becomes invalid after migration.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.invite_users(peer: impl Into&lt;PeerRef&gt;, user_ids: Vec&lt;i64&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Add one or more users to a chat. For channels all users are added in one request; for basic groups each user is added individually.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_history_ttl(peer: impl Into&lt;PeerRef&gt;, period: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set the auto-delete timer for a chat. <code>period</code> is in seconds. Common values: <code>86400</code> (1 day), <code>604800</code> (1 week), <code>2678400</code> (1 month). Pass <code>0</code> to disable.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_common_chats(user_id: i64, max_id: i64, limit: i32) → Result&lt;Vec&lt;tl::enums::Chat&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get chats shared between the logged-in account and <code>user_id</code>. Pass <code>max_id = 0</code> for the first page. Max <code>limit</code> is 100.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_online_count(peer: impl Into&lt;PeerRef&gt;) → Result&lt;i32, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the approximate number of members currently online in a group or channel.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_no_forwards(peer: impl Into&lt;PeerRef&gt;, enabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Enable or disable the no-forwards restriction. When enabled, members cannot forward messages out of this chat.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Client::parse_invite_hash(link: &str) → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">Parse the invite hash out of any <code>t.me/+…</code> or <code>t.me/joinchat/…</code> link format. Returns <code>None</code> if the link is not a valid invite.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_chat_theme(peer: impl Into&lt;PeerRef&gt;, emoticon: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Set the emoji-based colour theme for a chat. Pass a single emoji string such as <code>"🌈"</code> or <code>"❄️"</code> to apply a theme, or an empty string to reset to default.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_chat_reactions(peer: impl Into&lt;PeerRef&gt;, reactions: tl::enums::ChatReactions) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Control which reactions are allowed in a chat. Use <code>ChatReactionsAll</code>, <code>ChatReactionsSome</code>, or <code>ChatReactionsNone</code>.
<pre><code>// Allow all (including custom emoji)
client.set_chat_reactions(peer.clone(),
    tl::enums::ChatReactions::ChatReactionsAll(
        tl::types::ChatReactionsAll { allow_custom: true }
    )
).await?;

// Disable all reactions
client.set_chat_reactions(peer.clone(),
    tl::enums::ChatReactions::ChatReactionsNone
).await?;</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_forum(peer: impl Into&lt;PeerRef&gt;, enabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Enable or disable forum (topics) mode on a supergroup. Requires channel admin rights. Once enabled the group gains a <em>General</em> topic (ID 1) automatically. See <a href="./forum-topics.md">Forum Topics</a> for full topic management.</div>
</div>

---

## Advanced Messaging

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.forward_messages_returning(destination: impl Into&lt;PeerRef&gt;, message_ids: &[i32], source: impl Into&lt;PeerRef&gt;) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Like <code>forward_messages</code> but returns the newly created message objects in the destination chat. Useful when you need the forwarded message IDs immediately.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_reply_to_message(message: &IncomingMessage) → Result&lt;Option&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the message that <code>message</code> replies to. Returns <code>None</code> if it is not a reply or the original message is deleted/inaccessible.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_users_by_id(ids: &[i64]) → Result&lt;Vec&lt;Option&lt;User&gt;&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Bulk-fetch typed <code>User</code> objects by their IDs. The result is in the same order as <code>ids</code>; entries are <code>None</code> for deleted/unknown users.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_user_full(user_id: i64) → Result&lt;tl::types::UserFull, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the full info object for a user. Contains bio, common chats count, bot info, profile/fallback photos, privacy settings, pinned message ID, and more.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_pinned_message(peer: impl Into&lt;PeerRef&gt;) → Result&lt;Option&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch the currently pinned message in a chat. Returns <code>None</code> if nothing is pinned.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.pin_message(peer: impl Into&lt;PeerRef&gt;, message_id: i32, silent: bool, unpin: bool, pm_oneside: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Pin (or unpin) a message. Set <code>silent = true</code> to avoid a pin notification. <code>pm_oneside = true</code> pins only for the logged-in user in DMs.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unpin_message(peer: impl Into&lt;PeerRef&gt;, message_id: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Shorthand for unpinning a specific message (calls <code>pin_message</code> with <code>unpin = true</code>).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unpin_all_messages(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Unpin every pinned message in a chat at once.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_message_read_participants(peer: impl Into&lt;PeerRef&gt;, msg_id: i32) → Result&lt;Vec&lt;tl::types::ReadParticipantDate&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the list of users who have read a specific message and the time each read it. Only works in groups; returns an empty list for channels and private chats.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_replies(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, limit: i32, offset_id: i32) → Result&lt;Vec&lt;IncomingMessage&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Fetch thread replies under a message. <code>msg_id</code> is the root message ID. Pass <code>offset_id = 0</code> for the first page; use the lowest ID from the previous page to paginate. Max <code>limit</code> is 100.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_discussion_message(peer: impl Into&lt;PeerRef&gt;, msg_id: i32) → Result&lt;tl::types::messages::DiscussionMessage, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the linked discussion message in the comments group for a channel post. Returns the discussion metadata including unread counts and max-read IDs.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.read_discussion(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, read_max_id: i32) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Mark a discussion thread as read up to <code>read_max_id</code>. <code>peer</code> is the channel, <code>msg_id</code> is the root post.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_web_page_preview(text: impl Into&lt;String&gt;) → Result&lt;tl::enums::MessageMedia, InvocationError&gt;</span>
</div>
<div class="api-card-body">Generate a link preview for a URL embedded in <code>text</code>. Returns the <code>MessageMedia</code> that Telegram would attach (webpage card, article embed, video thumbnail, etc.).</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_media(peer: impl Into&lt;PeerRef&gt;, media: tl::enums::InputMedia) → Result&lt;tl::enums::MessageMedia, InvocationError&gt;</span>
</div>
<div class="api-card-body">Upload a media object to Telegram servers without sending it as a message. The returned <code>MessageMedia</code> can be reused in subsequent <code>send_message</code> calls via <code>InputMessage::copy_media()</code>. Distinct from <code>upload_file</code>  -  this works with an existing <code>InputMedia</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.export_message_link(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, grouped: bool, thread: bool) → Result&lt;String, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Get a <code>t.me/c/…</code> permalink for a message in a channel. <code>grouped = true</code> returns a link to the album group; <code>thread = true</code> links to the discussion thread. Only works for channels (not basic groups).
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_send_as_peers(peer: impl Into&lt;PeerRef&gt;) → Result&lt;Vec&lt;tl::enums::Peer&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">Get the list of identities the logged-in user can send messages as in a chat (e.g. own account, linked anonymous channel). Used to implement "send as channel" / anonymous posting.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_default_send_as(peer: impl Into&lt;PeerRef&gt;, send_as_peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body"><code>send_as_peer</code> must be one of the peers returned by <code>get_send_as_peers</code>. Sets the default identity for new messages in <code>peer</code>.</div>
</div>

---

## Translation & Transcription

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.translate_messages(peer: impl Into&lt;PeerRef&gt;, msg_ids: Vec&lt;i32&gt;, to_lang: impl Into&lt;String&gt;) → Result&lt;Vec&lt;tl::types::TextWithEntities&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Translate one or more messages to <code>to_lang</code> (e.g. <code>"en"</code>, <code>"ru"</code>). Returns translated text in the same order as <code>msg_ids</code>. Requires Telegram Premium for many language pairs.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.transcribe_audio(peer: impl Into&lt;PeerRef&gt;, msg_id: i32) → Result&lt;tl::types::messages::TranscribedAudio, InvocationError&gt;</span>
</div>
<div class="api-card-body">Request speech-to-text transcription of a voice message or video note. Transcription may be pending on first call; poll again if <code>result.pending == true</code>. Requires Telegram Premium.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.toggle_peer_translations(peer: impl Into&lt;PeerRef&gt;, disabled: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Show or hide the "Translate" toolbar button for a chat. <code>disabled = true</code> hides it.</div>
</div>

---

## Admin Log

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_admin_log(peer: impl Into&lt;PeerRef&gt;, query: impl Into&lt;String&gt;, limit: i32, max_id: i64, min_id: i64) → Result&lt;Vec&lt;tl::types::ChannelAdminLogEvent&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch the admin action log for a channel or supergroup. <code>query</code> filters by keyword (pass <code>""</code> for all events). Max <code>limit</code> is 100. Use <code>max_id</code> / <code>min_id</code> for pagination; pass <code>0</code> for both on the first call. Only works for channels/supergroups; returns an error for basic groups.
</div>
</div>

---

## Drafts

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.save_draft(peer: impl Into&lt;PeerRef&gt;, text: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Save a draft message for a chat. Pass an empty string to clear the current draft.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_all_drafts() → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Trigger a server push of all saved drafts across all chats. The server responds with <code>updateDraftMessage</code> updates in the update stream.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.clear_all_drafts() → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">Delete all saved drafts across all chats at once.</div>
</div>

