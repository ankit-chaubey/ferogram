/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

//! Chunked upload of encrypted file ciphertext for `messages.sendEncryptedFile`.

use ferogram_tl_types as tl;

use crate::error::E2eError;
use crate::transport::E2eTransport;

/// Re-label transport failures raised during upload.
fn upload_err(e: E2eError) -> E2eError {
    match e {
        E2eError::Transport(msg) => E2eError::UploadFailed(msg),
        other => other,
    }
}

/// Size of each uploaded part: 512 KiB.
const PART_SIZE: usize = 512 * 1024;
/// Files of at least 10 MiB use `upload.saveBigFilePart` (no MD5 checksum needed).
const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024;
/// Hard limit: 2 GiB.
const MAX_FILE_SIZE: usize = 2 * 1024 * 1024 * 1024;

/// Upload `ciphertext` to Telegram's file servers.
///
/// Returns the `InputEncryptedFile` reference to pass to `messages.sendEncryptedFile`.
/// `key_fingerprint` is computed by `crypto::file_key_fingerprint`.
///
/// If any part fails the whole upload must restart from part 0 (no resume).
pub async fn upload_encrypted_file<T: E2eTransport>(
    client: &T,
    ciphertext: &[u8],
    key_fingerprint: i32,
) -> Result<tl::enums::InputEncryptedFile, E2eError> {
    if ciphertext.len() > MAX_FILE_SIZE {
        return Err(E2eError::FileTooLarge);
    }

    let file_id: i64 = rand::random();
    let is_big = ciphertext.len() >= BIG_FILE_THRESHOLD;
    let total_parts = ciphertext.chunks(PART_SIZE).count() as i32;

    for (part_idx, chunk) in ciphertext.chunks(PART_SIZE).enumerate() {
        let part_no = part_idx as i32;
        let bytes = chunk.to_vec();

        if is_big {
            client
                .invoke(&tl::functions::upload::SaveBigFilePart {
                    file_id,
                    file_part: part_no,
                    file_total_parts: total_parts,
                    bytes,
                })
                .await
                .map_err(upload_err)?;
        } else {
            client
                .invoke(&tl::functions::upload::SaveFilePart {
                    file_id,
                    file_part: part_no,
                    bytes,
                })
                .await
                .map_err(upload_err)?;
        }
    }

    if is_big {
        Ok(tl::enums::InputEncryptedFile::BigUploaded(
            tl::types::InputEncryptedFileBigUploaded {
                id: file_id,
                parts: total_parts,
                key_fingerprint,
            },
        ))
    } else {
        let md5 = format!("{:x}", md5::compute(ciphertext));
        Ok(tl::enums::InputEncryptedFile::Uploaded(
            tl::types::InputEncryptedFileUploaded {
                id: file_id,
                parts: total_parts,
                md5_checksum: md5,
                key_fingerprint,
            },
        ))
    }
}
