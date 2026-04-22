# Contacts & Blocking

Methods for managing your contact list and blocking/unblocking users.

---

## Contacts

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_contacts() → Result&lt;Option&lt;Vec&lt;tl::enums::User&gt;&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch the full contact list. Returns <code>None</code> when the server indicates the contact list is unchanged since the last fetch (the server uses a hash-based caching scheme). In practice always returns <code>Some</code> on the first call.

```rust
if let Some(contacts) = client.get_contacts().await? {
    for c in contacts {
        if let tl::enums::User::User(u) = c {
            println!("{} {}", u.first_name.unwrap_or_default(), u.last_name.unwrap_or_default());
        }
    }
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.add_contact(user_id: i64, first_name: impl Into&lt;String&gt;, last_name: impl Into&lt;String&gt;, phone: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Add a user to your contact list. <code>phone</code> can be an empty string if you are adding by user ID rather than phone number. The name you supply is stored locally as your label for this contact, independent of the user's own profile name.

```rust
client.add_contact(user_id, "Alice", "Smith", "").await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_contacts(user_ids: Vec&lt;i64&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Remove one or more users from your contact list. Passing an empty vec is a no-op.

```rust
client.delete_contacts(vec![user_a, user_b]).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.search_contacts(query: impl Into&lt;String&gt;, limit: i32) → Result&lt;Vec&lt;tl::enums::Peer&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Search for users, groups, and channels by name. Searches across contacts, dialogs, and global results. Returns a merged, deduplicated list ordered by relevance.

```rust
let results = client.search_contacts("John", 20).await?;
```
</div>
</div>

---

## Import contacts

Import phone-number contacts in bulk. Each entry is `(phone, first_name, last_name)`. Returns the raw `ImportedContacts` result containing imported IDs and resolved user objects.

```rust
let result = client.import_contacts(&[
    ("+15550001234", "Alice", "Smith"),
    ("+15550005678", "Bob",   "Jones"),
]).await?;

println!("Imported {} contacts", result.imported.len());
for user in &result.users {
    println!("  resolved: {user:?}");
}
```

`result.retry_contacts` contains entries that could not be resolved (e.g. the number is not registered on Telegram).

---

## Blocking

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.block_user(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Block a user. Blocked users cannot send you messages, see your phone number, or add you to groups. The block also suppresses their stories from your feed.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.unblock_user(peer: impl Into&lt;PeerRef&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Remove a user from your block list.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_blocked_users(offset: i32, limit: i32) → Result&lt;Vec&lt;tl::enums::Peer&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Paginate through your block list. Start with <code>offset = 0</code>. The server caps <code>limit</code> at 100.

```rust
let mut offset = 0;
loop {
    let page = client.get_blocked_users(offset, 100).await?;
    if page.is_empty() { break; }
    offset += page.len() as i32;
    for peer in &page {
        println!("{peer:?}");
    }
}
```
</div>
</div>

---

## Full example

```rust
// Find and block all users named "Spammer"
let results = client.search_contacts("Spammer", 50).await?;
for peer in results {
    client.block_user(peer).await?;
    println!("Blocked {peer:?}");
}

// List current block list
let mut offset = 0;
loop {
    let page = client.get_blocked_users(offset, 100).await?;
    if page.is_empty() { break; }
    println!("Blocked users page: {page:?}");
    offset += page.len() as i32;
}
```
