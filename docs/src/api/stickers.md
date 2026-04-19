# Stickers

Methods for fetching, installing, and managing sticker sets, as well as resolving custom emoji.

See also: [`Sticker` type](../messaging/media.md#sticker-type) in the Media reference for how to receive and download sticker files from messages.

---

## Sticker sets

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_sticker_set(stickerset: tl::enums::InputStickerSet) → Result&lt;tl::types::messages::StickerSet, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch a sticker set and all its stickers. Pass an <code>InputStickerSet</code>  -  the most common variants are:

- `InputStickerSet::InputStickerSetShortName`  -  by <code>@short_name</code>
- `InputStickerSet::InputStickerSetID`  -  by numeric ID + access hash

```rust
let set = client.get_sticker_set(
    tl::enums::InputStickerSet::InputStickerSetShortName(
        tl::types::InputStickerSetShortName { short_name: "Animals".into() }
    )
).await?;

println!("Set: {}  -  {} stickers", set.set.title, set.set.count);
for doc in &set.documents {
    if let tl::enums::Document::Document(d) = doc {
        println!("  doc_id={}", d.id);
    }
}
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.install_sticker_set(stickerset: tl::enums::InputStickerSet, archived: bool) → Result&lt;tl::enums::messages::StickerSetInstallResult, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Install a sticker set for the current account. Pass <code>archived: true</code> to add it to the archive instead of the active set list.

The return value is <code>StickerSetInstallResult::Success</code> on a clean install, or <code>StickerSetInstallResult::Archive</code> when older sets were moved to the archive to make room.

```rust
let result = client.install_sticker_set(
    tl::enums::InputStickerSet::InputStickerSetShortName(
        tl::types::InputStickerSetShortName { short_name: "Animals".into() }
    ),
    false,
).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.uninstall_sticker_set(stickerset: tl::enums::InputStickerSet) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Remove a sticker set from the account's installed sets.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_all_stickers(hash: i64) → Result&lt;Option&lt;Vec&lt;tl::types::StickerSet&gt;&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
List all sticker sets installed for the current account. Pass <code>hash = 0</code> to always get the full list. Returns <code>None</code> when the server confirms the list is unchanged (hash match  -  used for caching).

```rust
if let Some(sets) = client.get_all_stickers(0).await? {
    for s in &sets {
        println!("{} ({})", s.title, s.short_name);
    }
}
```
</div>
</div>

---

## Custom emoji

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_custom_emoji_documents(document_ids: Vec&lt;i64&gt;) → Result&lt;Vec&lt;tl::enums::Document&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch the <code>Document</code> objects for a list of custom emoji IDs. Custom emoji IDs come from <code>MessageEntity::CustomEmoji { document_id }</code> when parsing formatted message text.

```rust
use ferogram::media::Document;

// Grab custom emoji IDs from a message's entities
let ids: Vec<i64> = msg.raw.entities.iter().flatten()
    .filter_map(|e| match e {
        tl::enums::MessageEntity::CustomEmoji(ce) => Some(ce.document_id),
        _ => None,
    })
    .collect();

let docs = client.get_custom_emoji_documents(ids).await?;
for doc in &docs {
    let d = Document::from_raw(match doc.clone() {
        tl::enums::Document::Document(d) => d,
        _ => continue,
    });
    println!("emoji doc {} mime={}", d.id(), d.mime_type());
}
```
</div>
</div>

---

## Sending a sticker

Stickers are sent as document media. Get the sticker's `Document`, build an `InputDocument`, and use `send_file`:

```rust
use ferogram::media::Sticker;

// From a received sticker message
if let Some(sticker) = Sticker::from_media(&incoming_msg.raw) {
    let input_media = tl::enums::InputMedia::InputMediaDocument(
        tl::types::InputMediaDocument {
            spoiler: false,
            optional_attributes: false,
            id: tl::enums::InputDocument::InputDocument(tl::types::InputDocument {
                id: sticker.id(),
                access_hash: sticker.access_hash(),   // from raw document
                file_reference: vec![],               // get from raw Document
            }),
            ttl_seconds: None,
            query: None,
        }
    );
    client.send_file(peer, input_media, "").await?;
}
```
