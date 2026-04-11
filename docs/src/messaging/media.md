# Media & Files

---

## Upload

```rust
// Upload from bytes: sequential
let uploaded: UploadedFile = client
    .upload_file("photo.jpg", &bytes)
    .await?;

// Upload from bytes: parallel chunks (faster for large files)
let uploaded = client
    .upload_file_concurrent("video.mp4", &bytes)
    .await?;

// Upload from an async reader (e.g. tokio::fs::File)
use tokio::fs::File;
let f = File::open("document.pdf").await?;
let uploaded = client.upload_stream("document.pdf", f).await?;
```

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
