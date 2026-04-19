# CDN Downloads

Large files on Telegram are often served from **CDN DCs**  -  lightweight edge data-centres that don't participate in the normal MTProto auth flow. ferogram exposes the full CDN download path via `CdnDownloader` in `ferogram::cdn_download`.

In normal usage you never need to interact with `CdnDownloader` directly  -  `download_media`, `download_media_concurrent`, and `iter_download` handle CDN redirects transparently. This page is for advanced use-cases where you need explicit control.

---

## How CDN redirects work

1. You call `upload.getFile` on the home DC.
2. If the file lives on a CDN DC, the server returns `upload.fileCdnRedirect` containing:
   - `dc_id`  -  which CDN DC to talk to
   - `file_token`  -  opaque credential for `upload.getCdnFile`
   - `encryption_key` (32 bytes AES-256-CTR key)
   - `encryption_iv` (16 bytes)
3. Connect to the CDN DC with `CdnDownloader::connect`.
4. Call `download_chunk_raw`, `download_all`, or `download_all_with_reupload`.
5. CDN DCs use **AES-256-CTR** (not AES-IGE). `CdnDownloader` decrypts transparently.

---

## Constants

```rust
use ferogram::cdn_download::CDN_CHUNK_SIZE;

// 131072 bytes  -  CDN DCs require exactly 128 KB fixed-size parts
// so the offset → hash mapping in upload.getCdnFileHashes stays consistent.
assert_eq!(CDN_CHUNK_SIZE, 128 * 1024);
```

---

## `CdnDownloader`

### Construction

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">CdnDownloader::connect(cdn_dc_addr: &str, cdn_dc_id: i16, file_token: Vec&lt;u8&gt;, encryption_key: [u8; 32], encryption_iv: [u8; 16], socks5: Option&lt;&Socks5Config&gt;) → Result&lt;Self, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Open a fresh connection to the CDN DC at <code>cdn_dc_addr</code> (format: <code>"ip:port"</code>) and return a ready downloader. Uses obfuscated transport. Pass the proxy config from your <code>Client</code> setup if one is active.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">CdnDownloader::new(conn: DcConnection, file_token: Vec&lt;u8&gt;, encryption_key: [u8; 32], encryption_iv: [u8; 16]) → Self</span>
</div>
<div class="api-card-body">
Wrap an already-open <code>DcConnection</code>. Useful if you manage connection pooling yourself.
</div>
</div>

### Downloading

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">cdn.download_chunk_raw(byte_offset: i64, limit: i32) → Result&lt;CdnChunkResult, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Download and AES-CTR-decrypt a single chunk at <code>byte_offset</code> with <code>limit</code> bytes. Use <code>CDN_CHUNK_SIZE</code> as the limit.

Returns one of:
- `CdnChunkResult::Data(Vec<u8>)`  -  decrypted bytes.
- `CdnChunkResult::ReuploadNeeded(Vec<u8>)`  -  server wants the file reuploaded to it first. Contains the `request_token`; call `upload.reuploadCdnFile` on the main DC then retry.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">cdn.download_all(total_size: Option&lt;i64&gt;) → Result&lt;Vec&lt;u8&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Download and reassemble the full file. If <code>ReuploadNeeded</code> is encountered, returns an error  -  use <code>download_all_with_reupload</code> instead if the file might need reuploading.

Pass <code>total_size = Some(n)</code> for pre-allocation; <code>None</code> is fine too.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">cdn.download_all_with_reupload&lt;F, Fut&gt;(total_size: Option&lt;i64&gt;, reupload_fn: F) → Result&lt;Vec&lt;u8&gt;, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Like <code>download_all</code> but handles <code>ReuploadNeeded</code> automatically. The <code>reupload_fn</code> closure receives the <code>request_token</code> bytes and must call <code>upload.reuploadCdnFile</code> on the main DC, then return <code>Ok(())</code>. The downloader retries the chunk after the reupload completes.

```rust
let bytes = cdn.download_all_with_reupload(
    Some(file_size),
    |request_token| async move {
        let body = serialize_reupload_cdn_file(&file_token, &request_token);
        main_dc_conn.rpc_call_raw(&body).await?;
        Ok(())
    },
).await?;
```
</div>
</div>

---

## Low-level TL helpers

These are public for callers who need to build raw requests manually:

```rust
use ferogram::cdn_download::{serialize_get_cdn_file, serialize_reupload_cdn_file};

// Build an upload.getCdnFile#395f69da payload
let req_bytes = serialize_get_cdn_file(&file_token, byte_offset, CDN_CHUNK_SIZE);

// Build an upload.reuploadCdnFile payload
let reup_bytes = serialize_reupload_cdn_file(&file_token, &request_token);
```

---

## Full example

```rust
use ferogram::cdn_download::{CdnDownloader, CDN_CHUNK_SIZE};

// 1. Detect the CDN redirect from an upload.getFile call (raw API)
// ...

// 2. Connect to the CDN DC
let mut cdn = CdnDownloader::connect(
    "149.154.167.222:443",  // CDN DC address from the redirect
    5,                       // CDN DC ID
    file_token,
    encryption_key,
    encryption_iv,
    None,  // no SOCKS5
).await?;

// 3. Download everything
let bytes = cdn.download_all(Some(total_size)).await?;

println!("Downloaded {} bytes", bytes.len());
```
