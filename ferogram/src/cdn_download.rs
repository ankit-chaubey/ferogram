//! Telegram CDN DC file downloads.
//!
//! Large files are served from CDN DCs. They use a separate, lightweight auth
//! flow and encrypt file data with **AES-256-CTR** (not AES-IGE).
//!
//! # Usage
//!
//! 1. Call `upload.getFile` on the main DC. If the file lives on a CDN DC,
//!    Telegram returns `upload.fileCdnRedirect` containing:
//!    - `dc_id` - which CDN DC hosts the file
//!    - `file_token` - opaque credential for `upload.getCdnFile`
//!    - `encryption_key` (32 bytes) and `encryption_iv` (16 bytes)
//!
//! 2. Connect to the CDN DC with [`CdnDownloader::connect`].
//!
//! 3. Call [`CdnDownloader::download_all`] or [`CdnDownloader::download_all_with_reupload`].

use crate::{InvocationError, TransportKind, dc_pool::DcConnection, socks5::Socks5Config};
use ferogram_crypto::aes::{ctr_crypt, ctr_iv_at_offset};

/// Chunk size for `upload.getCdnFile`.
///
/// CDN DCs require 128 KB fixed part size so that the offset → hash mapping
/// in `upload.getCdnFileHashes` remains consistent.
pub const CDN_CHUNK_SIZE: i32 = 128 * 1024;

/// A download session for a single file on a Telegram CDN DC.
pub struct CdnDownloader {
    conn: DcConnection,
    file_token: Vec<u8>,
    encryption_key: [u8; 32],
    encryption_iv: [u8; 16],
}

/// Result of [`CdnDownloader::download_chunk_raw`].
pub enum CdnChunkResult {
    /// Decrypted file bytes for this chunk.
    Data(Vec<u8>),
    /// Server requires `upload.reuploadCdnFile` on the main DC first.
    ReuploadNeeded(Vec<u8>),
}

impl CdnDownloader {
    /// Wrap an already-open CDN connection.
    pub fn new(
        conn: DcConnection,
        file_token: Vec<u8>,
        encryption_key: [u8; 32],
        encryption_iv: [u8; 16],
    ) -> Self {
        Self {
            conn,
            file_token,
            encryption_key,
            encryption_iv,
        }
    }

    /// Open a fresh connection to `cdn_dc_addr` ("ip:port") and return a ready downloader.
    pub async fn connect(
        cdn_dc_addr: &str,
        cdn_dc_id: i16,
        file_token: Vec<u8>,
        encryption_key: [u8; 32],
        encryption_iv: [u8; 16],
        socks5: Option<&Socks5Config>,
    ) -> Result<Self, InvocationError> {
        tracing::debug!("[cdn] Connecting to CDN DC{cdn_dc_id} at {cdn_dc_addr}");
        let conn = DcConnection::connect_raw(
            cdn_dc_addr,
            socks5,
            &TransportKind::Obfuscated { secret: None },
            cdn_dc_id,
        )
        .await?;
        Ok(Self::new(conn, file_token, encryption_key, encryption_iv))
    }

    // Core chunk download

    /// Download one chunk at `byte_offset` with `limit` bytes and decrypt it.
    pub async fn download_chunk_raw(
        &mut self,
        byte_offset: i64,
        limit: i32,
    ) -> Result<CdnChunkResult, InvocationError> {
        let body = serialize_get_cdn_file(&self.file_token, byte_offset, limit);
        let response = self.conn.rpc_call_raw(&body).await?;

        if response.len() < 4 {
            return Err(InvocationError::Deserialize(
                "CDN response too short".into(),
            ));
        }
        let cid = u32::from_le_bytes(response[..4].try_into().unwrap());

        match cid {
            // upload.cdnFile#a99fca4f bytes:bytes
            0xa99fca4f => {
                let mut bytes = tl_read_bytes(&response[4..])
                    .ok_or_else(|| InvocationError::Deserialize("cdn bytes decode".into()))?;
                let iv = ctr_iv_at_offset(&self.encryption_iv, byte_offset as u64);
                ctr_crypt(&mut bytes, &self.encryption_key, &iv);
                Ok(CdnChunkResult::Data(bytes))
            }
            // upload.cdnFileReuploadNeeded#eea8e46e request_token:bytes
            0xeea8e46e => {
                let request_token = tl_read_bytes(&response[4..])
                    .ok_or_else(|| InvocationError::Deserialize("cdn reupload token".into()))?;
                Ok(CdnChunkResult::ReuploadNeeded(request_token))
            }
            _ => Err(InvocationError::Deserialize(format!(
                "unexpected CDN constructor: {cid:#010x}"
            ))),
        }
    }

    // High-level download helpers

    /// Download the full file. Returns error if `cdnFileReuploadNeeded` is received.
    /// Use [`download_all_with_reupload`] if you need to handle reupload.
    pub async fn download_all(
        &mut self,
        total_size: Option<i64>,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut buf: Vec<u8> = total_size
            .map(|s| Vec::with_capacity(s as usize))
            .unwrap_or_default();
        let mut offset: i64 = 0;
        loop {
            match self.download_chunk_raw(offset, CDN_CHUNK_SIZE).await? {
                CdnChunkResult::Data(chunk) => {
                    if chunk.is_empty() {
                        break;
                    }
                    let len = chunk.len() as i64;
                    buf.extend_from_slice(&chunk);
                    offset += len;
                    if total_size.map(|t| offset >= t).unwrap_or(false)
                        || (len as i32) < CDN_CHUNK_SIZE
                    {
                        break;
                    }
                }
                CdnChunkResult::ReuploadNeeded(_) => {
                    return Err(InvocationError::Deserialize(
                        "cdnFileReuploadNeeded - use download_all_with_reupload".into(),
                    ));
                }
            }
        }
        Ok(buf)
    }

    /// Download the full file, automatically handling `cdnFileReuploadNeeded`.
    ///
    /// `reupload_fn` receives the `request_token` bytes and must call
    /// `upload.reuploadCdnFile` on the **main** DC (use [`serialize_reupload_cdn_file`]).
    pub async fn download_all_with_reupload<F, Fut>(
        &mut self,
        total_size: Option<i64>,
        mut reupload_fn: F,
    ) -> Result<Vec<u8>, InvocationError>
    where
        F: FnMut(Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<(), InvocationError>>,
    {
        let mut buf: Vec<u8> = total_size
            .map(|s| Vec::with_capacity(s as usize))
            .unwrap_or_default();
        let mut offset: i64 = 0;
        loop {
            match self.download_chunk_raw(offset, CDN_CHUNK_SIZE).await? {
                CdnChunkResult::Data(chunk) => {
                    if chunk.is_empty() {
                        break;
                    }
                    let len = chunk.len() as i64;
                    buf.extend_from_slice(&chunk);
                    offset += len;
                    if total_size.map(|t| offset >= t).unwrap_or(false)
                        || (len as i32) < CDN_CHUNK_SIZE
                    {
                        break;
                    }
                }
                CdnChunkResult::ReuploadNeeded(request_token) => {
                    tracing::debug!("[cdn] cdnFileReuploadNeeded - triggering reupload");
                    reupload_fn(request_token).await?;
                    // retry same offset
                }
            }
        }
        Ok(buf)
    }
}

// TL serialization helpers (public for callers building their own requests)

/// Serialize `upload.getCdnFile#395f69da`.
pub fn serialize_get_cdn_file(file_token: &[u8], offset: i64, limit: i32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x395f69da_u32.to_le_bytes());
    tl_write_bytes(&mut out, file_token);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&limit.to_le_bytes());
    out
}

/// Serialize `upload.reuploadCdnFile#9b2754a8`.
pub fn serialize_reupload_cdn_file(file_token: &[u8], request_token: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x9b2754a8_u32.to_le_bytes());
    tl_write_bytes(&mut out, file_token);
    tl_write_bytes(&mut out, request_token);
    out
}

fn tl_write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 254 {
        out.push(len as u8);
        out.extend_from_slice(data);
        let pad = (4 - (1 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    } else {
        out.push(0xfe);
        out.push((len & 0xff) as u8);
        out.push(((len >> 8) & 0xff) as u8);
        out.push(((len >> 16) & 0xff) as u8);
        out.extend_from_slice(data);
        let pad = (4 - (4 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
}

fn tl_read_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return Some(vec![]);
    }
    let (len, start) = if data[0] < 254 {
        (data[0] as usize, 1)
    } else if data.len() >= 4 {
        (
            data[1] as usize | (data[2] as usize) << 8 | (data[3] as usize) << 16,
            4,
        )
    } else {
        return None;
    };
    if data.len() < start + len {
        return None;
    }
    Some(data[start..start + len].to_vec())
}
