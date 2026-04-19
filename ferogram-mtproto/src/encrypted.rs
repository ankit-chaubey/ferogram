// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Encrypted MTProto 2.0 session (post auth-key).
//!
//! Once you have a `Finished` from [`crate::authentication`], construct an
//! [`EncryptedSession`] and use it to serialize/deserialize all subsequent
//! messages.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use ferogram_crypto::{AuthKey, DequeBuffer, decrypt_data_v2, encrypt_data_v2};
use ferogram_tl_types::RemoteCall;

/// Rolling deduplication buffer for server msg_ids.
const SEEN_MSG_IDS_MAX: usize = 500;
/// Maximum clock skew between client and server before a message is rejected.
const MSG_ID_TIME_WINDOW_SECS: i64 = 300;

/// Errors that can occur when decrypting a server message.
#[derive(Debug)]
pub enum DecryptError {
    /// The underlying crypto layer rejected the message.
    Crypto(ferogram_crypto::DecryptError),
    /// The decrypted inner message was too short to contain a valid header.
    FrameTooShort,
    /// Session-ID mismatch (possible replay or wrong connection).
    SessionMismatch,
    /// Server msg_id is outside the ±300 s window of corrected local time.
    MsgIdTimeWindow,
    /// This msg_id was already seen in the rolling 500-entry buffer.
    DuplicateMsgId,
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Crypto(e) => write!(f, "crypto: {e}"),
            Self::FrameTooShort => write!(f, "inner plaintext too short"),
            Self::SessionMismatch => write!(f, "session_id mismatch"),
            Self::MsgIdTimeWindow => write!(f, "server msg_id outside ±300 s time window"),
            Self::DuplicateMsgId => write!(f, "duplicate server msg_id (replay)"),
        }
    }
}
impl std::error::Error for DecryptError {}

/// The inner payload extracted from a successfully decrypted server frame.
pub struct DecryptedMessage {
    /// `salt` sent by the server.
    pub salt: i64,
    /// The `session_id` from the frame.
    pub session_id: i64,
    /// The `msg_id` of the inner message.
    pub msg_id: i64,
    /// `seq_no` of the inner message.
    pub seq_no: i32,
    /// TL-serialized body of the inner message.
    pub body: Vec<u8>,
}

/// Shared, persistent dedup ring for server msg_ids.
///
/// Outlives individual `EncryptedSession` objects so that replayed frames
/// from a prior connection cycle are still rejected after reconnect.
pub type SeenMsgIds = std::sync::Arc<std::sync::Mutex<VecDeque<i64>>>;

/// Allocate a fresh seen-msg_id ring.
pub fn new_seen_msg_ids() -> SeenMsgIds {
    std::sync::Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        SEEN_MSG_IDS_MAX,
    )))
}

/// MTProto 2.0 encrypted session state.
pub struct EncryptedSession {
    auth_key: AuthKey,
    session_id: i64,
    sequence: i32,
    last_msg_id: i64,
    /// Current server salt to include in outgoing messages.
    pub salt: i64,
    /// Clock skew in seconds vs. server.
    pub time_offset: i32,
    /// Rolling 500-entry dedup buffer of seen server msg_ids.
    /// Shared with the owning DcConnection so it survives reconnects.
    seen_msg_ids: SeenMsgIds,
}

impl EncryptedSession {
    /// Create a new encrypted session from the output of `authentication::finish`.
    ///
    /// `seen_msg_ids` should be the persistent ring owned by the `DcConnection`
    /// (or any other owner that outlives individual sessions).  Pass
    /// `new_seen_msg_ids()` for the very first connection on a slot.
    pub fn new(auth_key: [u8; 256], first_salt: i64, time_offset: i32) -> Self {
        Self::with_seen(auth_key, first_salt, time_offset, new_seen_msg_ids())
    }

    /// Like `new` but reuses an existing seen-msg_id ring (reconnect path).
    pub fn with_seen(
        auth_key: [u8; 256],
        first_salt: i64,
        time_offset: i32,
        seen_msg_ids: SeenMsgIds,
    ) -> Self {
        let mut rnd = [0u8; 8];
        getrandom::getrandom(&mut rnd).expect("getrandom");
        Self {
            auth_key: AuthKey::from_bytes(auth_key),
            session_id: i64::from_le_bytes(rnd),
            sequence: 0,
            last_msg_id: 0,
            salt: first_salt,
            time_offset,
            seen_msg_ids,
        }
    }

    /// Return a clone of the shared seen-msg_id ring for passing to a
    /// replacement session on reconnect.
    pub fn seen_msg_ids(&self) -> SeenMsgIds {
        std::sync::Arc::clone(&self.seen_msg_ids)
    }

    /// Compute the next message ID (based on corrected server time).
    fn next_msg_id(&mut self) -> i64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        // Keep arithmetic in u64: seconds since epoch with time_offset applied.
        let secs = now.as_secs().wrapping_add(self.time_offset as i64 as u64);
        let nanos = now.subsec_nanos() as u64;
        let mut id = ((secs << 32) | (nanos << 2)) as i64;
        if self.last_msg_id >= id {
            id = self.last_msg_id + 4;
        }
        self.last_msg_id = id;
        id
    }

    /// Next content-related seq_no (odd) and advance the counter.
    /// Used for all regular RPC requests.
    fn next_seq_no(&mut self) -> i32 {
        let n = self.sequence * 2 + 1;
        self.sequence += 1;
        n
    }

    /// Return the current even seq_no WITHOUT advancing the counter.
    ///
    /// Service messages (MsgsAck, containers, etc.) MUST use an even seqno
    /// per the MTProto spec so the server does not expect a reply.
    pub fn next_seq_no_ncr(&self) -> i32 {
        self.sequence * 2
    }

    /// Correct the outgoing sequence counter when the server reports a
    /// `bad_msg_notification` with error codes 32 (seq_no too low) or
    /// 33 (seq_no too high).
    ///
    pub fn correct_seq_no(&mut self, code: u32) {
        match code {
            32 => {
                // seq_no too low: jump forward so next send is well above server expectation
                self.sequence += 64;
                log::debug!(
                    "[ferogram] seq_no correction: code 32, bumped seq to {}",
                    self.sequence
                );
            }
            33 => {
                // seq_no too high: step back, but never below 1 to avoid
                // re-using seq_no=1 which was already sent this session.
                // Zeroing would make the next content message get seq_no=1,
                // which the server already saw and will reject again with code 32.
                self.sequence = self.sequence.saturating_sub(16).max(1);
                log::debug!(
                    "[ferogram] seq_no correction: code 33, lowered seq to {}",
                    self.sequence
                );
            }
            _ => {}
        }
    }

    /// Undo the last `next_seq_no` increment.
    ///
    /// Called before retrying a request after `bad_server_salt` so the resent
    /// message uses the same seq_no slot rather than advancing the counter a
    /// second time (which would produce seq_no too high → bad_msg_notification
    /// code 33 → server closes TCP → early eof).
    pub fn undo_seq_no(&mut self) {
        self.sequence = self.sequence.saturating_sub(1);
    }

    /// Re-derive the clock skew from a server-provided `msg_id`.
    ///
    /// Called on `bad_msg_notification` error codes 16 (msg_id too low) and
    /// 17 (msg_id too high) so clock drift is corrected at any point in the
    /// session, not only at connect time.
    ///
    pub fn correct_time_offset(&mut self, server_msg_id: i64) {
        // Upper 32 bits of msg_id = Unix seconds on the server
        let server_time = (server_msg_id >> 32) as i32;
        let local_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i32;
        let new_offset = server_time.wrapping_sub(local_now);
        log::debug!(
            "[ferogram] time_offset correction: {} → {} (server_time={server_time})",
            self.time_offset,
            new_offset
        );
        self.time_offset = new_offset;
        // Seed last_msg_id from the server's msg_id (bits 1-0 cleared to 0b00)
        // so the next next_msg_id() call produces a strictly larger value.
        self.last_msg_id = (server_msg_id & !0x3i64).max(self.last_msg_id);
    }

    /// Allocate a fresh `(msg_id, seqno)` pair for an inner container message
    /// WITHOUT encrypting anything.
    ///
    /// `content_related = true`  → odd seqno, advances counter  (regular RPCs)
    /// `content_related = false` → even seqno, no advance       (MsgsAck, container)
    ///
    pub fn alloc_msg_seqno(&mut self, content_related: bool) -> (i64, i32) {
        let msg_id = self.next_msg_id();
        let seqno = if content_related {
            self.next_seq_no()
        } else {
            self.next_seq_no_ncr()
        };
        (msg_id, seqno)
    }

    /// Encrypt a pre-serialized TL body into a wire-ready MTProto frame.
    ///
    /// `content_related` controls whether the seqno is odd (content, advances
    /// the counter) or even (service, no advance).
    ///
    /// Returns `(encrypted_wire_bytes, msg_id)`.
    /// Used for (bad_msg re-send) and (container inner messages).
    pub fn pack_body_with_msg_id(&mut self, body: &[u8], content_related: bool) -> (Vec<u8>, i64) {
        let msg_id = self.next_msg_id();
        let seq_no = if content_related {
            self.next_seq_no()
        } else {
            self.next_seq_no_ncr()
        };

        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());

        encrypt_data_v2(&mut buf, &self.auth_key);
        (buf.as_ref().to_vec(), msg_id)
    }

    /// Encrypt a pre-built `msg_container` body (the container itself is
    /// a non-content-related message with an even seqno).
    ///
    /// Returns `(encrypted_wire_bytes, container_msg_id)`.
    /// The container_msg_id is needed so callers can map it back to inner
    /// requests when a bad_msg_notification or bad_server_salt arrives for
    /// the container rather than the individual inner message.
    ///
    pub fn pack_container(&mut self, container_body: &[u8]) -> (Vec<u8>, i64) {
        self.pack_body_with_msg_id(container_body, false)
    }

    /// Encrypt `body` using a **caller-supplied** `msg_id` instead of generating one.
    ///
    /// Required by `auth.bindTempAuthKey`, which must use the same `msg_id`
    /// in both the outer MTProto envelope and the inner `bind_auth_key_inner`.
    pub fn pack_body_at_msg_id(&mut self, body: &[u8], msg_id: i64) -> Vec<u8> {
        let seq_no = self.next_seq_no();
        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());
        encrypt_data_v2(&mut buf, &self.auth_key);
        buf.as_ref().to_vec()
    }

    /// Serialize and encrypt a TL function into a wire-ready byte vector.
    pub fn pack_serializable<S: ferogram_tl_types::Serializable>(&mut self, call: &S) -> Vec<u8> {
        let body = call.to_bytes();
        let msg_id = self.next_msg_id();
        let seq_no = self.next_seq_no();

        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());

        encrypt_data_v2(&mut buf, &self.auth_key);
        buf.as_ref().to_vec()
    }

    /// Like `pack_serializable` but also returns the `msg_id`.
    pub fn pack_serializable_with_msg_id<S: ferogram_tl_types::Serializable>(
        &mut self,
        call: &S,
    ) -> (Vec<u8>, i64) {
        let body = call.to_bytes();
        let msg_id = self.next_msg_id();
        let seq_no = self.next_seq_no();
        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());
        encrypt_data_v2(&mut buf, &self.auth_key);
        (buf.as_ref().to_vec(), msg_id)
    }

    /// Like [`pack`] but also returns the `msg_id` allocated for this message.
    pub fn pack_with_msg_id<R: RemoteCall>(&mut self, call: &R) -> (Vec<u8>, i64) {
        let body = call.to_bytes();
        let msg_id = self.next_msg_id();
        let seq_no = self.next_seq_no();
        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());
        encrypt_data_v2(&mut buf, &self.auth_key);
        (buf.as_ref().to_vec(), msg_id)
    }

    /// Encrypt and frame a [`RemoteCall`] into a ready-to-send MTProto message.
    pub fn pack<R: RemoteCall>(&mut self, call: &R) -> Vec<u8> {
        let body = call.to_bytes();
        let msg_id = self.next_msg_id();
        let seq_no = self.next_seq_no();

        let inner_len = 8 + 8 + 8 + 4 + 4 + body.len();
        let mut buf = DequeBuffer::with_capacity(inner_len, 32);
        buf.extend(self.salt.to_le_bytes());
        buf.extend(self.session_id.to_le_bytes());
        buf.extend(msg_id.to_le_bytes());
        buf.extend(seq_no.to_le_bytes());
        buf.extend((body.len() as u32).to_le_bytes());
        buf.extend(body.iter().copied());

        encrypt_data_v2(&mut buf, &self.auth_key);
        buf.as_ref().to_vec()
    }

    /// Decrypt an encrypted server frame.
    pub fn unpack(&self, frame: &mut [u8]) -> Result<DecryptedMessage, DecryptError> {
        let plaintext = decrypt_data_v2(frame, &self.auth_key).map_err(DecryptError::Crypto)?;

        if plaintext.len() < 32 {
            return Err(DecryptError::FrameTooShort);
        }

        let salt = i64::from_le_bytes(plaintext[..8].try_into().unwrap());
        let session_id = i64::from_le_bytes(plaintext[8..16].try_into().unwrap());
        let msg_id = i64::from_le_bytes(plaintext[16..24].try_into().unwrap());
        let seq_no = i32::from_le_bytes(plaintext[24..28].try_into().unwrap());
        let body_len = u32::from_le_bytes(plaintext[28..32].try_into().unwrap()) as usize;

        if session_id != self.session_id {
            return Err(DecryptError::SessionMismatch);
        }

        // Check server time (upper 32 bits of msg_id) against ±300 s window.
        // Warn and continue: clock self-corrects via bad_msg_notification or pong.
        let server_secs = (msg_id as u64 >> 32) as i64;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let corrected = now + self.time_offset as i64;
        if (server_secs - corrected).abs() > MSG_ID_TIME_WINDOW_SECS {
            log::warn!(
                "[ferogram] msg_id time-window violation: server_secs={server_secs} \
                 corrected_local={corrected} skew={}s: processing anyway, \
                 clock will self-correct via bad_msg_notification/pong",
                (server_secs - corrected).abs()
            );
        }

        // Rolling 500-entry dedup.
        {
            let mut seen = self.seen_msg_ids.lock().unwrap();
            if seen.contains(&msg_id) {
                return Err(DecryptError::DuplicateMsgId);
            }
            seen.push_back(msg_id);
            if seen.len() > SEEN_MSG_IDS_MAX {
                seen.pop_front();
            }
        }

        // Maximum body length: 16 MB.
        if body_len > 16 * 1024 * 1024 {
            return Err(DecryptError::FrameTooShort);
        }
        if 32 + body_len > plaintext.len() {
            return Err(DecryptError::FrameTooShort);
        }
        // MTProto 2.0: minimum padding is 12 bytes; no upper bound.
        let padding = plaintext.len() - 32 - body_len;
        if padding < 12 {
            return Err(DecryptError::FrameTooShort);
        }
        let body = plaintext[32..32 + body_len].to_vec();

        Ok(DecryptedMessage {
            salt,
            session_id,
            msg_id,
            seq_no,
            body,
        })
    }

    /// Return the auth_key bytes (for persistence).
    pub fn auth_key_bytes(&self) -> [u8; 256] {
        self.auth_key.to_bytes()
    }

    /// Return the current session_id.
    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Reset session state: new random session_id, zeroed seq_no and last_msg_id,
    /// cleared dedup buffer.
    ///
    /// Called on `bad_msg_notification` error codes 32/33 (seq_no mismatch).
    /// Creates a new session_id and resets seq_no to avoid persistent desync.
    pub fn reset_session(&mut self) {
        let mut rnd = [0u8; 8];
        getrandom::getrandom(&mut rnd).expect("getrandom");
        let old_session = self.session_id;
        self.session_id = i64::from_le_bytes(rnd);
        self.sequence = 0;
        self.last_msg_id = 0;
        // Do not clear seen_msg_ids: the ring is shared with the owning
        // DcConnection and must survive session resets to reject replayed frames.
        log::debug!(
            "[ferogram] session reset: {:#018x} → {:#018x}",
            old_session,
            self.session_id
        );
    }
}

impl EncryptedSession {
    /// Like [`decrypt_frame`] but also performs seen-msg_id deduplication using the
    /// supplied ring.  Pass `&self.inner.seen_msg_ids` from the client.
    pub fn decrypt_frame_dedup(
        auth_key: &[u8; 256],
        session_id: i64,
        frame: &mut [u8],
        seen: &SeenMsgIds,
    ) -> Result<DecryptedMessage, DecryptError> {
        let msg = Self::decrypt_frame_with_offset(auth_key, session_id, frame, 0)?;
        {
            let mut s = seen.lock().unwrap();
            if s.contains(&msg.msg_id) {
                return Err(DecryptError::DuplicateMsgId);
            }
            s.push_back(msg.msg_id);
            if s.len() > SEEN_MSG_IDS_MAX {
                s.pop_front();
            }
        }
        Ok(msg)
    }

    /// Decrypt a frame using explicit key + session_id: no mutable state needed.
    /// Used by the split-reader task so it can decrypt without locking the writer.
    /// `time_offset` is the session's current clock skew (seconds); pass 0 if unknown.
    pub fn decrypt_frame(
        auth_key: &[u8; 256],
        session_id: i64,
        frame: &mut [u8],
    ) -> Result<DecryptedMessage, DecryptError> {
        Self::decrypt_frame_with_offset(auth_key, session_id, frame, 0)
    }

    /// Like [`decrypt_frame`] but applies the time-window check with the given
    /// `time_offset` (seconds, server_time − local_time).
    pub fn decrypt_frame_with_offset(
        auth_key: &[u8; 256],
        session_id: i64,
        frame: &mut [u8],
        time_offset: i32,
    ) -> Result<DecryptedMessage, DecryptError> {
        let key = AuthKey::from_bytes(*auth_key);
        let plaintext = decrypt_data_v2(frame, &key).map_err(DecryptError::Crypto)?;
        if plaintext.len() < 32 {
            return Err(DecryptError::FrameTooShort);
        }
        let salt = i64::from_le_bytes(plaintext[..8].try_into().unwrap());
        let sid = i64::from_le_bytes(plaintext[8..16].try_into().unwrap());
        let msg_id = i64::from_le_bytes(plaintext[16..24].try_into().unwrap());
        let seq_no = i32::from_le_bytes(plaintext[24..28].try_into().unwrap());
        let body_len = u32::from_le_bytes(plaintext[28..32].try_into().unwrap()) as usize;
        if sid != session_id {
            return Err(DecryptError::SessionMismatch);
        }
        // Warn but continue: clock self-corrects via bad_msg_notification or pong.
        let server_secs = (msg_id as u64 >> 32) as i64;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let corrected = now + time_offset as i64;
        if (server_secs - corrected).abs() > MSG_ID_TIME_WINDOW_SECS {
            log::warn!(
                "[ferogram] msg_id time-window violation (split-reader): server_secs={server_secs} \
                 corrected_local={corrected} skew={}s: processing anyway",
                (server_secs - corrected).abs()
            );
        }
        // Maximum body length: 16 MB.
        if body_len > 16 * 1024 * 1024 {
            return Err(DecryptError::FrameTooShort);
        }
        if 32 + body_len > plaintext.len() {
            return Err(DecryptError::FrameTooShort);
        }
        // MTProto 2.0: minimum padding is 12 bytes; no upper bound.
        let padding = plaintext.len() - 32 - body_len;
        if padding < 12 {
            return Err(DecryptError::FrameTooShort);
        }
        let body = plaintext[32..32 + body_len].to_vec();
        Ok(DecryptedMessage {
            salt,
            session_id: sid,
            msg_id,
            seq_no,
            body,
        })
    }
}
