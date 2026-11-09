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

//! Error type for the secret chat layer.

use thiserror::Error;

/// Errors returned by `ferogram-e2e`.
#[derive(Debug, Error)]
pub enum E2eError {
    /// A DH value (`g_a`, `g_b`) failed the range checks against the prime.
    #[error("DH value outside valid range")]
    DhValidationFailed,

    /// The shared secret computed by the DH exchange is not usable.
    #[error("computed shared secret is invalid")]
    SharedSecretInvalid,

    /// The key fingerprint on a message or exchange does not match our key.
    #[error("key fingerprint mismatch")]
    KeyFingerprintMismatch,

    /// A message or file could not be decrypted or failed an integrity check.
    #[error("decryption failed")]
    DecryptionFailed,

    /// A message or file could not be encrypted.
    #[error("encryption failed")]
    EncryptionFailed,

    /// No secret chat with this id is known.
    #[error("chat not found: {0}")]
    ChatNotFound(i32),

    /// A secret chat with this id already exists.
    #[error("chat already exists: {0}")]
    ChatAlreadyExists(i32),

    /// The chat is not in a state that allows this operation.
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// A text message or attachment exceeds the size limit (text: 512 KiB).
    #[error("message too large")]
    MessageTooLarge,

    /// A key re-exchange is already in progress for this chat.
    #[error("re-key already in progress")]
    RekeyInProgress,

    /// Serializing an inner TL object failed.
    #[error("TL serialization failed")]
    TlSerializationFailed,

    /// Parsing an inner TL object failed.
    #[error("TL deserialization failed")]
    TlDeserializationFailed,

    /// A re-key security check failed (fingerprint mismatch on accept or commit).
    #[error("security error: {0}")]
    SecurityError(String),

    /// The underlying client failed to complete an RPC.
    #[error("transport: {0}")]
    Transport(String),

    /// Uploading an encrypted file failed.
    #[error("file upload failed: {0}")]
    UploadFailed(String),

    /// The file exceeds the 2 GB limit.
    #[error("file too large (max 2 GB)")]
    FileTooLarge,
}
