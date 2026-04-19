# Invite Links

Full API for creating, editing, revoking, and managing chat invite links, as well as handling join requests.

---

## Create & export

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.export_invite_link(peer: impl Into&lt;PeerRef&gt;, expire_date: Option&lt;i32&gt;, usage_limit: Option&lt;i32&gt;, request_needed: bool) → Result&lt;tl::enums::ExportedChatInvite, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Create a new invite link for a chat.

- `expire_date`  -  Unix timestamp after which the link stops working. Pass `None` for no expiry.
- `usage_limit`  -  Maximum number of times the link can be used. Pass `None` for unlimited.
- `request_needed`  -  If `true`, users who join via this link must be approved by an admin before entering.

```rust
// Permanent link, up to 50 uses
let inv = client.export_invite_link(peer.clone(), None, Some(50), false).await?;

// Link that expires in 24 hours, requires approval
let tomorrow = (std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).unwrap()
    .as_secs() + 86400) as i32;
let inv = client.export_invite_link(peer.clone(), Some(tomorrow), None, true).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">Client::parse_invite_hash(link: &str) → Option&lt;&str&gt;</span>
</div>
<div class="api-card-body">
Extract the raw invite hash from any <code>t.me/+…</code> or <code>t.me/joinchat/…</code> link format. Returns <code>None</code> if the string is not a valid invite link.

```rust
let hash = Client::parse_invite_hash("https://t.me/+AbCdEfGhIj");
// => Some("AbCdEfGhIj")
```
</div>
</div>

---

## Revoke & edit

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.revoke_invite_link(peer: impl Into&lt;PeerRef&gt;, link: impl Into&lt;String&gt;) → Result&lt;tl::enums::ExportedChatInvite, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Revoke an existing invite link immediately. After revocation the link stops accepting new joins. Returns the updated invite object showing the revoked state. The link is not deleted  -  it still appears in history and can be deleted with <code>delete_invite_link</code>.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.edit_invite_link(peer: impl Into&lt;PeerRef&gt;, link: impl Into&lt;String&gt;, expire_date: Option&lt;i32&gt;, usage_limit: Option&lt;i32&gt;, request_needed: bool) → Result&lt;tl::enums::ExportedChatInvite, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Update an existing invite link's properties. Only non-permanent links can be edited. The same fields as <code>export_invite_link</code> apply.
</div>
</div>

---

## List & delete

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_invite_links(peer: impl Into&lt;PeerRef&gt;, admin_id: i64, revoked: bool, limit: i32) → Result&lt;Vec&lt;tl::enums::ExportedChatInvite&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
List invite links created by a specific admin. Pass <code>revoked: false</code> for active links, <code>true</code> for revoked ones. Maximum <code>limit</code> is 100.

```rust
// All active links created by the logged-in user (user_id from get_me)
let me = client.get_me().await?;
let links = client.get_invite_links(peer.clone(), me.id, false, 100).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_invite_link(peer: impl Into&lt;PeerRef&gt;, link: impl Into&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Permanently delete a revoked invite link. The link must already be revoked before it can be deleted.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_revoked_invite_links(peer: impl Into&lt;PeerRef&gt;, admin_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Bulk-delete all revoked links created by <code>admin_id</code> in one call. Useful for cleaning up the invite history.
</div>
</div>

---

## Join requests

When a link is created with `request_needed: true`, users who click it appear as pending join requests that an admin must approve or reject.

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.approve_join_request(peer: impl Into&lt;PeerRef&gt;, user_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Approve a single pending join request. The user is added to the chat immediately.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.reject_join_request(peer: impl Into&lt;PeerRef&gt;, user_id: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Reject and dismiss a single pending join request.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.approve_all_join_requests(peer: impl Into&lt;PeerRef&gt;, link: Option&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Approve all pending join requests at once. If <code>link</code> is <code>Some(url)</code>, only requests submitted via that specific link are approved. Pass <code>None</code> to approve all pending requests regardless of which link they came from.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.reject_all_join_requests(peer: impl Into&lt;PeerRef&gt;, link: Option&lt;String&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Reject all pending join requests at once, with the same optional link filter as <code>approve_all_join_requests</code>.
</div>
</div>

---

## Inspect link usage

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_invite_link_members(peer: impl Into&lt;PeerRef&gt;, link: Option&lt;String&gt;, requested: bool, limit: i32) → Result&lt;tl::types::messages::ChatInviteImporters, InvocationError&gt;</span>
</div>
<div class="api-card-body">
List members who joined via a specific invite link, or all pending join requesters.

- `link`  -  the invite URL. `None` to query across all links.
- `requested: false`  -  users who already joined.
- `requested: true`  -  users with a pending join request still awaiting approval.

```rust
// Who is waiting for approval?
let pending = client
    .get_invite_link_members(peer.clone(), None, true, 50)
    .await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_admins_with_invites(peer: impl Into&lt;PeerRef&gt;) → Result&lt;tl::types::messages::ChatAdminsWithInvites, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Get a breakdown of each admin's invite link counts (active, revoked). Useful for auditing which admins created how many links.
</div>
</div>

---

## Full example: approval-gated invite

```rust
// Create link that requires admin approval
let inv = client
    .export_invite_link(peer.clone(), None, None, true)
    .await?;

println!("Share this link: {}", match &inv {
    tl::enums::ExportedChatInvite::Invite(i) => &i.link,
    _ => "",
});

// Later: check who is waiting
let pending = client
    .get_invite_link_members(peer.clone(), None, true, 100)
    .await?;

for importer in &pending.importers {
    println!("Approving user {}", importer.user_id);
    client.approve_join_request(peer.clone(), importer.user_id).await?;
}
```
