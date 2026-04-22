# Media & Files

---

## Upload

ferogram provides three upload methods. Choose based on file size and where
the data comes from.

| Method | Input | Best for |
|---|---|---|
| `upload_file` | `&[u8]` | Small files already in memory (under ~10 MB) |
| `upload_file_concurrent` | `Arc<Vec<u8>>` | Large files already in memory (10 MB+), parallel chunks |
| `upload_stream` | `impl AsyncRead` | Files on disk or any async reader |

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_file(name: &str, data: &[u8]) → Result&lt;UploadedFile, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Upload a byte slice sequentially. Uses a single worker and sends chunks one at
a time. Suitable for files under ~10 MB or when parallelism is not needed.

```rust
let bytes: Vec<u8> = std::fs::read("photo.jpg")?;
let uploaded = client.upload_file("photo.jpg", &bytes).await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_file_concurrent(name: &str, data: Arc&lt;Vec&lt;u8&gt;&gt;, mime_type: &str) → Result&lt;UploadedFile, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Upload using a parallel worker pool. Takes <code>Arc&lt;Vec&lt;u8&gt;&gt;</code> for
zero-copy sharing across worker tasks. Significantly faster for large files
(video, large documents) because multiple chunks are in flight simultaneously.

```rust
use std::sync::Arc;

let bytes = Arc::new(std::fs::read("video.mp4")?);
let uploaded = client
    .upload_file_concurrent("video.mp4", bytes, "video/mp4")
    .await?;
```
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.upload_stream&lt;R: AsyncRead + Unpin&gt;(reader: &amp;mut R, name: &str, mime_type: &str) → Result&lt;UploadedFile, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Upload from any type that implements <code>tokio::io::AsyncRead</code>. Reads
the entire reader into memory, then automatically delegates to
<code>upload_file_concurrent</code> for large data or <code>upload_file</code>
for small data based on a built-in threshold.

This is the most convenient method when working with files on disk, network
streams, or in-memory cursors.

### Parameters

| Param | Type | Description |
|---|---|---|
| `reader` | `&mut impl AsyncRead + Unpin` | Any async reader: `tokio::fs::File`, `tokio::io::BufReader`, `std::io::Cursor`, etc. |
| `name` | `&str` | Filename that Telegram stores and shows to recipients. |
| `mime_type` | `&str` | MIME type. Pass `""` to auto-detect from the file extension in `name`. |

### Examples

```rust
// Upload from a file on disk
use tokio::fs::File;

let mut f = File::open("document.pdf").await?;
let uploaded = client
    .upload_stream(&mut f, "document.pdf", "application/pdf")
    .await?;
```

```rust
// Auto-detect MIME type from the filename extension
let mut f = File::open("photo.jpg").await?;
let uploaded = client
    .upload_stream(&mut f, "photo.jpg", "")   // "" -> auto-detects image/jpeg
    .await?;
```

```rust
// Upload from an in-memory buffer via std::io::Cursor
use std::io::Cursor;
use tokio::io::BufReader;

let data: Vec<u8> = generate_pdf_bytes();
let mut reader = BufReader::new(Cursor::new(data));
let uploaded = client
    .upload_stream(&mut reader, "report.pdf", "application/pdf")
    .await?;
```

```rust
// Upload a file produced by an async process (e.g. compression)
use tokio::io::AsyncWriteExt;

let (reader, mut writer) = tokio::io::duplex(64 * 1024);
tokio::spawn(async move {
    // write compressed data to writer ...
    writer.shutdown().await.unwrap();
});

let mut r = reader;
let uploaded = client
    .upload_stream(&mut r, "archive.gz", "application/gzip")
    .await?;
```

### Notes

- `upload_stream` reads the entire reader to completion before uploading.
  There is no true streaming upload to Telegram; the data must fit in memory.
- For data already in memory as `Vec<u8>`, prefer `upload_file` or
  `upload_file_concurrent` to avoid the extra `read_to_end` allocation.
- The MIME type is used by Telegram clients to decide how to display the file
  (inline image preview, audio player, generic document, etc.). Passing `""`
  enables built-in MIME detection from the file extension.
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

## Send file

```rust
// Send as document (false) or as photo/media (true)
client.send_file(peer.clone(), uploaded, false).await?;

// Send as album (multiple files in one message group)
client.send_album(peer.clone(), vec![uploaded_a, uploaded_b]).await?;
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
let bytes: Vec<u8> = client.download_media(&msg_media).await?;

// To bytes: parallel chunks
let bytes = client.download_media_concurrent(&msg_media).await?;

// Stream to file
client.download_media_to_file(&msg_media, "output.jpg").await?;

// Via Downloadable trait (Photo, Document, Sticker)
let bytes = client.download(&photo).await?;
```

### `DownloadIter`: streaming chunks

```rust
let location = msg.raw.download_location().unwrap();
let mut iter = client.iter_download(location);
iter = iter.chunk_size(128 * 1024); // 128 KB chunks

while let Some(chunk) = iter.next().await? {
    file.write_all(&chunk).await?;
}
```

| Method | Description |
|---|---|
| `client.iter_download(location)` | Create a lazy chunk iterator |
| `iter.chunk_size(bytes)` | Set download chunk size |
| `iter.next()` | `async → Option<Vec<u8>>` |

---

## `Photo` type

```rust
use ferogram::media::Photo;

let photo = Photo::from_media(&msg.raw).unwrap();
// or
let photo = msg.photo().unwrap();

photo.id()                // i64
photo.access_hash()       // i64
photo.date()              // i32: Unix timestamp
photo.has_stickers()      // bool
photo.largest_thumb_type() // &str: e.g. "y", "x", "s"

let bytes = client.download(&photo).await?;
```

| Constructor | Description |
|---|---|
| `Photo::from_raw(tl::types::Photo)` | Wrap raw TL photo |
| `Photo::from_media(&MessageMedia)` | Extract from message media |

---

## `Document` type

```rust
use ferogram::media::Document;

let doc = Document::from_media(&msg.raw).unwrap();
// or
let doc = msg.document().unwrap();

doc.id()              // i64
doc.access_hash()     // i64
doc.date()            // i32
doc.mime_type()       // &str
doc.size()            // i64: bytes
doc.file_name()       // Option<&str>
doc.is_animated()     // bool: animated GIF or sticker

let bytes = client.download(&doc).await?;
```

| Constructor | Description |
|---|---|
| `Document::from_raw(tl::types::Document)` | Wrap raw TL document |
| `Document::from_media(&MessageMedia)` | Extract from message media |

---

## `Sticker` type

```rust
use ferogram::media::Sticker;

let sticker = Sticker::from_media(&msg.raw).unwrap();

sticker.id()          // i64
sticker.mime_type()   // &str: "image/webp" or "video/webm"
sticker.emoji()       // Option<&str>: associated emoji
sticker.is_video()    // bool: animated video sticker

let bytes = client.download(&sticker).await?;
```

| Constructor | Description |
|---|---|
| `Sticker::from_document(Document)` | Wrap a document as a sticker |
| `Sticker::from_media(&MessageMedia)` | Extract sticker from message |

---

## `Downloadable` trait

`Photo`, `Document`, and `Sticker` all implement `Downloadable`, so you can use `client.download(&item)` on any of them uniformly.

```rust
use ferogram::media::Downloadable;

async fn save_any<D: Downloadable>(client: &Client, item: &D) -> Vec<u8> {
    client.download(item).await.unwrap()
}
```

---

## Download location from message

```rust
// Get an InputFileLocation from the raw message
use ferogram::media::download_location_from_media;

if let Some(loc) = download_location_from_media(&msg.raw) {
    let bytes = client.download_media(&loc).await?;
}

// Or via IncomingMessage convenience:
msg.download_media("output.jpg").await?;
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
