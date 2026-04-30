# Profile & Account

Methods for updating your own profile, managing active sessions, and controlling account-level settings.

---

## Profile

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_profile(first_name: Option&lt;String&gt;, last_name: Option&lt;String&gt;, about: Option&lt;String&gt;) → Result&lt;tl::enums::User, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Change your display name and/or bio. Pass <code>None</code> for any field you want to leave unchanged. Returns the updated <code>User</code> object.

```rust
// Change just the bio
client.update_profile(None, None, Some("🦀 Rust developer".to_string())).await?;

// Change full name
client.update_profile(
    Some("Alice".to_string()),
    Some("Smith".to_string()),
    None,
).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_username(username: impl Into&lt;String&gt;) → Result&lt;tl::enums::User, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set or change your @username. Pass an empty string to remove the username. Returns the updated <code>User</code> object. Telegram will return an error if the username is already taken or violates naming rules.

```rust
client.update_username("my_new_handle").await?;
client.update_username("").await?;  // remove username
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_status(offline: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Manually set your online/offline status. Pass <code>offline: false</code> to appear online, <code>true</code> to mark yourself as offline. Telegram resets online status automatically after ~5 minutes of inactivity, so call this periodically if you need a persistent "online" appearance.
</div>
</div>

---

## Profile photo

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_profile_photo(file: UploadedFile) → Result&lt;tl::enums::Photo, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set or add a new profile photo. Upload the image with <code>upload_file</code> first.

```rust
let bytes = tokio::fs::read("avatar.jpg").await?;
let uploaded = client.upload_file(&bytes, "avatar.jpg", "image/jpeg").await?;
let photo = client.set_profile_photo(uploaded).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_profile_photos(photo_ids: Vec&lt;(i64, i64, Vec&lt;u8&gt;)&gt;) → Result&lt;Vec&lt;i64&gt;, InvocationError&gt;</span>
</div>
<parameter name="file_text"># Profile & Account

Methods for updating your own profile, managing active sessions, and controlling account-level settings.

---

## Profile

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_profile(first_name: Option&lt;String&gt;, last_name: Option&lt;String&gt;, about: Option&lt;String&gt;) → Result&lt;tl::enums::User, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Change your display name and/or bio. Pass <code>None</code> for any field you want to leave unchanged. Returns the updated <code>User</code> object.

```rust
// Change just the bio
client.update_profile(None, None, Some("🦀 Rust developer".to_string())).await?;

// Change full name
client.update_profile(
    Some("Alice".to_string()),
    Some("Smith".to_string()),
    None,
).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_username(username: impl Into&lt;String&gt;) → Result&lt;tl::enums::User, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set or change your @username. Pass an empty string to remove the username. Returns the updated <code>User</code> object. Telegram will return an error if the username is already taken or violates naming rules.

```rust
client.update_username("my_new_handle").await?;
client.update_username("").await?;  // remove username
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_status(offline: bool) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Manually set your online/offline status. Pass <code>offline: false</code> to appear online, <code>true</code> to mark yourself as offline. Telegram resets online status automatically after ~5 minutes of inactivity.
</div>
</div>

---

## Profile photo

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_profile_photos(peer: impl Into&lt;PeerRef&gt;, limit: i32) → Result&lt;Vec&lt;tl::enums::Photo&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch up to <code>limit</code> profile photos for a user. Only works on user peers; passing a chat or channel returns an error.

```rust
let photos = client.get_profile_photos(peer, 10).await?;
for p in photos {
    if let tl::enums::Photo::Photo(photo) = p {
        println!("photo id={}", photo.id);
    }
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.iter_profile_photos(peer: impl Into&lt;PeerRef&gt;, chunk_size: i32) → ProfilePhotoIter</span>
</div>
<div class="api-card-body">
Returns a lazy iterator over all profile photos for a user. <code>chunk_size</code> controls how many are fetched per page (pass <code>0</code> for the default). Call <code>.total_count()</code> on the iterator to get the total before iterating.

```rust
let mut iter = client.iter_profile_photos(peer, 0).await?;
if let Some(total) = iter.total_count() {
    println!("{total} photos total");
}
while let Some(photo) = iter.next(&client).await? {
    // handle photo
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_profile_photo(file: UploadedFile) → Result&lt;tl::enums::Photo, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Set or add a new profile photo. Upload the image with <code>upload_file</code> first. Returns the new <code>Photo</code> object.

```rust
let bytes = tokio::fs::read("avatar.jpg").await?;
let uploaded = client.upload_file(&bytes, "avatar.jpg", "image/jpeg").await?;
let photo = client.set_profile_photo(uploaded).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.delete_profile_photos(photo_ids: Vec&lt;(i64, i64, Vec&lt;u8&gt;)&gt;) → Result&lt;Vec&lt;i64&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Delete one or more profile photos. Each tuple is <code>(photo_id, access_hash, file_reference)</code>  -  all three come from a <code>tl::types::Photo</code>. Returns the IDs of successfully deleted photos.

```rust
// Get photos via iter_profile_photos, then delete
let tl::enums::Photo::Photo(p) = photo else { return; };
client.delete_profile_photos(vec![(p.id, p.access_hash, p.file_reference.clone())]).await?;
```
</div>
</div>

---

## Sessions

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_authorizations() → Result&lt;Vec&lt;tl::types::Authorization&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
List all active login sessions for this account. Each <code>Authorization</code> contains:

| Field | Type | Description |
|---|---|---|
| `hash` | `i64` | Session identifier  -  pass to `terminate_session` |
| `device_model` | `String` | e.g. `"iPhone 15"`, `"Chrome"` |
| `platform` | `String` | e.g. `"iOS"`, `"Linux"` |
| `app_name` | `String` | Client app name |
| `date_created` | `i32` | Unix timestamp |
| `date_active` | `i32` | Last-seen Unix timestamp |
| `ip` | `String` | IP address of the session |
| `country` | `String` | Country code |
| `current` | `bool` | Whether this is the current session |

```rust
let sessions = client.get_authorizations().await?;
for s in &sessions {
    println!("{}  -  {}  -  active: {}", s.device_model, s.ip, s.date_active);
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.terminate_session(hash: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Revoke a specific session by its <code>hash</code> from <code>get_authorizations</code>. The device is immediately logged out.

```rust
let sessions = client.get_authorizations().await?;
for s in sessions {
    if !s.current && s.app_name.contains("WebK") {
        // terminate old web sessions
        client.terminate_session(s.hash).await?;
    }
}
```
</div>
</div>

---

## Full example: profile refresh

```rust
// Update name and bio at once
client.update_profile(
    Some("Bot".to_string()),
    Some("Account".to_string()),
    Some("Powered by ferogram 🦀".to_string()),
).await?;

// Set a new avatar
let bytes = tokio::fs::read("new_avatar.png").await?;
let f = client.upload_file(&bytes, "avatar.png", "image/png").await?;
client.set_profile_photo(f).await?;

// Go offline
client.update_status(true).await?;
```

---

## Emoji status

Set or clear the animated emoji shown next to the logged-in user's name (Telegram Premium feature):

```rust
// Set an emoji status using a custom emoji document ID
// (obtain IDs from sticker sets via client.get_sticker_set)
client.set_emoji_status(Some(5260885697911948121), None).await?;

// Set with an expiry (Unix timestamp)
client.set_emoji_status(Some(5260885697911948121), Some(1_800_000_000)).await?;

// Clear the current emoji status
client.set_emoji_status(None, None).await?;
```

`document_id` is the `id` field from a `tl::types::Document` belonging to a custom-emoji sticker. Pass `None` to remove the status. `until` is an optional Unix timestamp after which the status expires automatically; pass `None` for no expiry.
