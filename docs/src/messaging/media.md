# Media & Files

---

## Upload

ferogram provides three upload methods. Choose based on file size and where
the data comes from.

| Method | Input | Best for |
|---|---|---|
| `upload_file(path)` | path on disk | Any file - stat → chunked upload |
| `upload(source, name)` | `impl AsyncRead` | In-memory bytes or any async reader |

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_file(path: impl AsRef&lt;Path&gt;) → Result&lt;UploadedFile, InvocationError&gt;</span>

Upload a file from disk. Stats the file for optimal part sizing, then uploads in chunks. Automatically uses parallel workers and `saveBigFilePart` for files over 10 MB.

```rust
let uploaded = client.upload_file("/tmp/photo.jpg").await?;
client.send_media(peer, InputMessage::media(uploaded.as_photo_media())).await?;
```

---

<span class="api-card-sig">client.upload(source: impl AsyncRead + Unpin + Send, name: &str) → Result&lt;UploadedFile, InvocationError&gt;</span>

Upload from any `AsyncRead` source (in-memory `Cursor`, network stream, etc.). Buffers the stream to determine size, then uploads with optimal part sizing.

```rust
// from a Vec<u8>
use std::io::Cursor;
let uploaded = client.upload(Cursor::new(bytes), "photo.jpg").await?;

// from a tokio File
let f = tokio::fs::File::open("video.mp4").await?;
let uploaded = client.upload(f, "video.mp4").await?;
```

</div>
<div class="api-card-body">
Upload a byte slice sequentially. Uses a single worker and sends chunks one at
a time. Suitable for files under ~10 MB or when parallelism is not needed.

```rust
let bytes: Vec<u8> = std::fs::read("photo.jpg")?;
let uploaded = client.upload_file("/tmp/photo.jpg").await?;
```
</div>
</div>

---

### `UploadedFile` methods

| Method | Return | Description |
|---|---|---|
| `uploaded.name()` | `&str` | Original filename |
| `uploaded.mime_type()` | `&str` | Detected MIME type |
| `uploaded.as_document_media()` | `tl::enums::InputMedia` | Ready to send as document |
| `uploaded.as_photo_media()` | `tl::enums::InputMedia` | Ready to send as photo |

---

## Upload media (reusable)

Upload a file to Telegram's servers and get back an `InputMedia` handle that can be reused in multiple sends without re-uploading:

```rust
use ferogram::InputMessage;

let uploaded = client.upload_file(&bytes, "photo.jpg", "image/jpeg").await?;
let media = uploaded.as_photo_media();

// Upload to Telegram's servers (no message sent)
let stored = client.upload_media("@peer", media.clone()).await?;

// Reuse the stored media handle
let msg = InputMessage::text("Here it is!").copy_media(stored.into_input_media());
client.send_message("@peer", msg).await?;
```

---

## Send file

```rust
use ferogram::InputMessage;

// Send a file as document or photo
let uploaded = client.upload_file(&bytes, "photo.jpg", "image/jpeg").await?;
client.send_file("@peer", uploaded.as_photo_media(), &InputMessage::text("Caption")).await?;

// Or attach via InputMessage
let msg = InputMessage::text("Here is the file")
    .copy_media(uploaded.as_document_media());
client.send_message("@peer", msg).await?;

// Send as album (multiple files in one message group)
client.send_album("@peer", vec![
    uploaded_a.as_photo_media(),
    uploaded_b.as_photo_media(),
]).await?;
```

### `AlbumItem`: per-item control in albums

```rust
use ferogram::media::AlbumItem;

let items = vec![
    AlbumItem::new(uploaded_a.as_photo_media())
        .caption("First photo 📸"),
    AlbumItem::new(uploaded_b.as_document_media())
        .caption("The report 📄")
        .reply_to(Some(msg_id)),
];
client.send_album(peer.clone(), items).await?;
```

| Method | Description |
|---|---|
| `AlbumItem::new(media)` | Wrap an `InputMedia` |
| `.caption(str)` | Caption text for this item |
| `.reply_to(Option<i32>)` | Reply to message ID |

---

## Download

```rust
// To bytes: sequential
```rust
let mut buf = Vec::new();
client.download(msg.media().unwrap(), &mut buf).await?;

client.download_file(msg.media().unwrap(), "/tmp/output.jpg").await?;

let bytes = msg.bytes().await?;                        // → Vec<u8>

let mut file = tokio::fs::File::create("out.mp4").await?;
msg.download(&mut file).await?;                        // stream to file

if let Some(mut iter) = client.iter_download(msg.media().unwrap()) {
    while let Some(chunk) = iter.next().await? {
        // chunk: bytes::Bytes - zero-copy slice
        process(&chunk);
    }
}
```

| Method | Dest | Returns |
|--------|------|---------|
| `client.download(media, dest)` | any `AsyncWrite` | `u64` bytes written |
| `client.download_file(media, path)` | file on disk | `u64` bytes written |
| `client.iter_download(media)` | caller-driven | `Option<DownloadIter>` |
| `msg.download(dest)` | any `AsyncWrite` | `u64` bytes written |
| `msg.bytes()` | in-memory `Vec<u8>` | `Vec<u8>` |

## `Downloadable` trait
## `Downloadable` trait

`Photo`, `Document`, and `Sticker` all implement `Downloadable`, so you can use `client.download_item(&item)` (internal) on any of them uniformly.
For the public API, pass `msg.media()` to `client.download`.

```rust
use ferogram::media::Downloadable;

async fn save_any<D: Downloadable>(client: &Client, item: &D) -> Vec<u8> {
    // internal method - public API uses client.download(media, dest)
    client.download(item).await.unwrap()
}
```

---

## Download location from message

```rust
// Get an InputFileLocation from the raw message
use ferogram::media::download_location_from_media;

// use msg.bytes() or client.download(msg.media().unwrap(), &mut buf) instead

// Or via IncomingMessage convenience:
client.download_file(msg.media().unwrap(), "output.jpg").await?;
```

---

## Media groups (albums)

When Telegram delivers a grouped media send (album), each message in the group carries the same `grouped_id`. To fetch all messages belonging to the same album as a known message ID:

```rust
let msgs = client.get_media_group("@mychannel", 42).await?;
println!("{} messages in this album", msgs.len());

for m in &msgs {
    if let Some(photo) = m.photo() {
        println!("  photo id={}", photo.id());
    } else if let Some(doc) = m.document() {
        println!("  document mime={}", doc.mime_type().unwrap_or("?"));
    }
}
```

`get_media_group` accepts any peer and a message ID that is part of the album. It returns all messages in the group including the seed message. For non-channel chats the server returns only the single message.

### Detecting albums in the update stream

```rust
if let Update::NewMessage(msg) = update {
    if msg.grouped_id().is_some() {
        // This message is part of an album; you can call
        // client.get_media_group(msg.peer_id(), msg.id()).await
        // to retrieve the full set.
    }
}
```
