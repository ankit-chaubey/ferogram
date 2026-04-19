# Privacy & Notifications

Methods for reading and writing Telegram account privacy rules and per-chat notification settings.

---

## Privacy rules

Privacy rules control who can see your phone number, last seen, profile photo, etc.

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_privacy(key: tl::enums::InputPrivacyKey) → Result&lt;Vec&lt;tl::enums::PrivacyRule&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch the current privacy rules for a specific setting identified by <code>key</code>.

### Privacy keys

| Variant | Controls |
|---|---|
| `InputPrivacyKey::StatusTimestamp` | Last seen / online status |
| `InputPrivacyKey::ChatInvite` | Who can add you to groups |
| `InputPrivacyKey::Call` | Who can call you |
| `InputPrivacyKey::ProfilePhoto` | Who sees your profile photo |
| `InputPrivacyKey::PhoneNumber` | Who sees your phone number |
| `InputPrivacyKey::ForwardedMessages` | Who can link forwards to your account |
| `InputPrivacyKey::PhoneCall` | Who can voice-call you |
| `InputPrivacyKey::PhoneP2P` | Peer-to-peer call mode |
| `InputPrivacyKey::Voices` | Voice messages |
| `InputPrivacyKey::About` | Who sees your bio |

```rust
let rules = client.get_privacy(
    tl::enums::InputPrivacyKey::StatusTimestamp
).await?;

for rule in &rules {
    println!("{rule:?}");
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.set_privacy(key: tl::enums::InputPrivacyKey, rules: Vec&lt;tl::enums::InputPrivacyRule&gt;) → Result&lt;Vec&lt;tl::enums::PrivacyRule&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Update the privacy rules for a key. Rules are evaluated in order  -  the first matching rule wins.

### Rule variants

| Variant | Meaning |
|---|---|
| `InputPrivacyValueAllowAll` | Allow everyone |
| `InputPrivacyValueAllowContacts` | Allow contacts only |
| `InputPrivacyValueAllowUsers { users }` | Allow specific users |
| `InputPrivacyValueDisallowAll` | Block everyone |
| `InputPrivacyValueDisallowContacts` | Block contacts |
| `InputPrivacyValueDisallowUsers { users }` | Block specific users |

```rust
use tl::enums::{InputPrivacyKey, InputPrivacyRule};

// Last seen: contacts only
client.set_privacy(
    InputPrivacyKey::StatusTimestamp,
    vec![
        InputPrivacyRule::InputPrivacyValueAllowContacts,
        InputPrivacyRule::InputPrivacyValueDisallowAll,
    ],
).await?;

// Phone: nobody
client.set_privacy(
    InputPrivacyKey::PhoneNumber,
    vec![InputPrivacyRule::InputPrivacyValueDisallowAll],
).await?;

// Profile photo: everyone
client.set_privacy(
    InputPrivacyKey::ProfilePhoto,
    vec![InputPrivacyRule::InputPrivacyValueAllowAll],
).await?;
```
</div>
</div>

---

## Notification settings

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_notify_settings(peer: impl Into&lt;PeerRef&gt;) → Result&lt;tl::enums::PeerNotifySettings, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Get the notification settings for a specific chat.

```rust
let settings = client.get_notify_settings(peer.clone()).await?;
if let tl::enums::PeerNotifySettings::PeerNotifySettings(s) = settings {
    println!("muted until: {:?}", s.mute_until);
    println!("sound: {:?}", s.sound);
    println!("show_previews: {:?}", s.show_previews);
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.update_notify_settings(peer: impl Into&lt;PeerRef&gt;, settings: tl::enums::InputPeerNotifySettings) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Update notification settings for a chat. Only the fields you set are changed on the server; unset <code>Option</code> fields are left as-is.

### `InputPeerNotifySettings` fields

| Field | Type | Description |
|---|---|---|
| `mute_until` | `Option<i32>` | Unix timestamp to mute until. `Some(i32::MAX)` = forever. `Some(0)` = unmute. |
| `sound` | `Option<NotificationSound>` | Sound to play |
| `show_previews` | `Option<bool>` | Show message preview in notification |
| `silent` | `Option<bool>` | Deliver silently without sound |

```rust
use tl::enums::InputPeerNotifySettings;
use tl::types::InputPeerNotifySettings as S;

// Mute a chat for 1 hour
let until = (std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).unwrap()
    .as_secs() + 3600) as i32;

client.update_notify_settings(
    peer.clone(),
    tl::enums::InputPeerNotifySettings::InputPeerNotifySettings(S {
        show_previews: None,
        silent: None,
        mute_until: Some(until),
        sound: None,
        stories_muted: None,
        stories_hide_sender: None,
        stories_sound: None,
    }),
).await?;

// Unmute
client.update_notify_settings(
    peer.clone(),
    tl::enums::InputPeerNotifySettings::InputPeerNotifySettings(S {
        mute_until: Some(0),
        show_previews: None, silent: None, sound: None,
        stories_muted: None, stories_hide_sender: None, stories_sound: None,
    }),
).await?;
```
</div>
</div>

---

## Common privacy recipes

```rust
// "Ghost mode"  -  hide everything from non-contacts
use tl::enums::{InputPrivacyKey as Key, InputPrivacyRule as Rule};

let ghost = vec![Rule::InputPrivacyValueAllowContacts, Rule::InputPrivacyValueDisallowAll];

client.set_privacy(Key::StatusTimestamp, ghost.clone()).await?;
client.set_privacy(Key::ProfilePhoto,    ghost.clone()).await?;
client.set_privacy(Key::PhoneNumber,     vec![Rule::InputPrivacyValueDisallowAll]).await?;
client.set_privacy(Key::ChatInvite,      ghost.clone()).await?;
```
