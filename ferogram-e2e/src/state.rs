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

//! Per-chat state and the store that keeps it (in memory, optionally on disk).
//!
//! # Store file format
//!
//! The optional on-disk store (conventionally `<session_name>.e2e`) is a small
//! binary file, all integers little-endian:
//!
//! ```text
//! magic    "FGE2E\0"            6 bytes
//! version  u8                   currently 1
//! count    u32                  number of chats
//! chat*    count records        see `SecretChatState::encode`
//! checksum SHA-256 of all preceding bytes (32 bytes)
//! ```
//!
//! The file is replaced atomically (write to a temp file, then rename) and, on
//! Unix, created with mode `0600`, because it holds every chat's `auth_key`.
//! Only established chats are stored; pending exchanges and the resend buffer
//! live in memory only.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::error::E2eError;

/// Progress of a key re-exchange (forward secrecy) for one chat.
#[derive(Debug, Clone)]
pub enum RekeyingState {
    /// No re-key in progress.
    Idle,
    /// We sent `RequestKey` and wait for the peer's `AcceptKey`.
    Requested {
        /// Identifier of this exchange.
        exchange_id: i64,
    },
    /// We sent `AcceptKey` and wait for the peer's `CommitKey`.
    Accepted {
        /// Identifier of this exchange.
        exchange_id: i64,
    },
}

/// Maximum number of sent message blobs retained for Resend handling.
const SENT_BUFFER_CAPACITY: usize = 100;

/// Per-chat state stored in memory (and optionally persisted to disk).
#[derive(Debug, Clone)]
pub struct SecretChatState {
    /// Secret chat id assigned by Telegram.
    pub id: i32,
    /// Access hash for `InputEncryptedChat`.
    pub access_hash: i64,
    /// Shared 256-byte key from the DH exchange. Written unencrypted to the store file.
    pub auth_key: [u8; 256],
    /// true = we initiated this chat (User A).
    pub admin: bool,
    /// Peer user ID.
    pub user_id: i64,
    /// Raw incoming sequence counter.
    pub in_seq_no: i32,
    /// Raw outgoing sequence counter.
    pub out_seq_no: i32,
    /// Parity bit for incoming wire encoding.
    pub in_seq_no_x: i32,
    /// Parity bit for outgoing wire encoding.
    pub out_seq_no_x: i32,
    /// Remote peer's supported layer (updated on NotifyLayer receipt).
    pub layer: i32,
    /// Per-message TTL in seconds (0 = no TTL).
    pub ttl: i32,
    /// Messages until next re-key.
    pub ttr: i32,
    /// Unix epoch seconds of last auth_key rotation (for 7-day re-key timer).
    pub updated_at_secs: u64,
    /// MTProto version in use (1 or 2).
    pub mtproto: u8,
    /// Where the current re-key exchange stands.
    pub rekeying_state: RekeyingState,
    /// Keyed by exchange_id.
    /// Requested state: stores 256-byte private key.
    /// Accepted state: stores 256-byte computed auth_key.
    pub temp_rekey_map: HashMap<i64, Vec<u8>>,
    /// Ring buffer of (wire_out_seq_no, encrypted_blob) for Resend support.
    /// Not persisted: only valid for the current session.
    pub sent: VecDeque<(i32, Vec<u8>)>,
}

impl SecretChatState {
    /// Create the state for a freshly established chat.
    ///
    /// `admin` is `true` on the initiating side; it decides the sequence-number
    /// parity bits. The layer defaults to [`crate::dh::DEFAULT_LAYER`] until the
    /// peer announces its own.
    pub fn new(id: i32, access_hash: i64, auth_key: [u8; 256], admin: bool, user_id: i64) -> Self {
        let (in_seq_no_x, out_seq_no_x) = if admin { (0, 1) } else { (1, 0) };
        Self {
            id,
            access_hash,
            auth_key,
            admin,
            user_id,
            in_seq_no: 0,
            out_seq_no: 0,
            in_seq_no_x,
            out_seq_no_x,
            layer: crate::dh::DEFAULT_LAYER,
            ttl: 0,
            ttr: 100,
            updated_at_secs: now_secs(),
            mtproto: 2,
            rekeying_state: RekeyingState::Idle,
            temp_rekey_map: HashMap::new(),
            sent: VecDeque::new(),
        }
    }

    /// Wire-encoded incoming seq no: raw * 2 + x.
    pub fn wire_in_seq_no(&self) -> i32 {
        self.in_seq_no * 2 + self.in_seq_no_x
    }

    /// Wire-encoded outgoing seq no: raw * 2 + x.
    pub fn wire_out_seq_no(&self) -> i32 {
        self.out_seq_no * 2 + self.out_seq_no_x
    }

    /// Seconds since last auth_key rotation.
    pub fn secs_since_update(&self) -> u64 {
        now_secs().saturating_sub(self.updated_at_secs)
    }

    /// Stamp updated_at_secs to now, reset ttr, swap in the new key.
    pub fn rotate_key(&mut self, new_key: [u8; 256]) {
        self.auth_key = new_key;
        self.ttr = 100;
        self.updated_at_secs = now_secs();
    }

    /// Buffer an outgoing encrypted blob for potential Resend.
    /// `wire_seq_no` is the wire_out_seq_no that was used when this blob was sent.
    pub fn push_sent(&mut self, wire_seq_no: i32, blob: Vec<u8>) {
        self.sent.push_back((wire_seq_no, blob));
        while self.sent.len() > SENT_BUFFER_CAPACITY {
            self.sent.pop_front();
        }
    }

    /// Look up sent blobs in the range [start_wire, end_wire] (inclusive, step 2).
    pub fn lookup_sent(&self, start_wire: i32, end_wire: i32) -> Vec<(i32, Vec<u8>)> {
        self.sent
            .iter()
            .filter(|(seq, _)| *seq >= start_wire && *seq <= end_wire)
            .cloned()
            .collect()
    }
}

/// Current Unix time in seconds (0 if the clock is before the epoch).
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// File magic of the store format.
const STORE_MAGIC: &[u8; 6] = b"FGE2E\0";
/// Current store format version.
const STORE_VERSION: u8 = 1;
/// Length of the trailing SHA-256 checksum.
const CHECKSUM_LEN: usize = 32;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("e2e store: {msg}"))
}

/// Bounds-checked little-endian reader over a byte slice.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("overflow"))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| invalid("truncated"))?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

impl SecretChatState {
    /// Append the binary record of this chat to `out`.
    ///
    /// Layout: `id i32, access_hash i64, auth_key [256], admin u8, user_id i64,
    /// in_seq_no i32, out_seq_no i32, in_seq_no_x i32, out_seq_no_x i32,
    /// layer i32, ttl i32, ttr i32, updated_at_secs u64, mtproto u8,
    /// rekey tag u8 (0 idle, 1 requested, 2 accepted) [+ exchange_id i64],
    /// temp_rekey_map: count u32, then (exchange_id i64, len u32, bytes)*`.
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.access_hash.to_le_bytes());
        out.extend_from_slice(&self.auth_key);
        out.push(self.admin as u8);
        out.extend_from_slice(&self.user_id.to_le_bytes());
        for v in [
            self.in_seq_no,
            self.out_seq_no,
            self.in_seq_no_x,
            self.out_seq_no_x,
            self.layer,
            self.ttl,
            self.ttr,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.updated_at_secs.to_le_bytes());
        out.push(self.mtproto);
        match self.rekeying_state {
            RekeyingState::Idle => out.push(0),
            RekeyingState::Requested { exchange_id } => {
                out.push(1);
                out.extend_from_slice(&exchange_id.to_le_bytes());
            }
            RekeyingState::Accepted { exchange_id } => {
                out.push(2);
                out.extend_from_slice(&exchange_id.to_le_bytes());
            }
        }
        out.extend_from_slice(&(self.temp_rekey_map.len() as u32).to_le_bytes());
        for (exchange_id, bytes) in &self.temp_rekey_map {
            out.extend_from_slice(&exchange_id.to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
    }

    /// Parse one chat record written by [`SecretChatState::encode`].
    fn decode(r: &mut Reader<'_>) -> io::Result<Self> {
        let id = r.i32()?;
        let access_hash = r.i64()?;
        let mut auth_key = [0u8; 256];
        auth_key.copy_from_slice(r.take(256)?);
        let admin = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("bad admin flag")),
        };
        let user_id = r.i64()?;
        let in_seq_no = r.i32()?;
        let out_seq_no = r.i32()?;
        let in_seq_no_x = r.i32()?;
        let out_seq_no_x = r.i32()?;
        let layer = r.i32()?;
        let ttl = r.i32()?;
        let ttr = r.i32()?;
        let updated_at_secs = r.u64()?;
        let mtproto = r.u8()?;
        let rekeying_state = match r.u8()? {
            0 => RekeyingState::Idle,
            1 => RekeyingState::Requested {
                exchange_id: r.i64()?,
            },
            2 => RekeyingState::Accepted {
                exchange_id: r.i64()?,
            },
            _ => return Err(invalid("bad rekey state")),
        };
        let map_len = r.u32()? as usize;
        // Each entry is at least 12 bytes; reject counts the file cannot hold.
        if map_len > r.remaining() / 12 {
            return Err(invalid("bad rekey map length"));
        }
        let mut temp_rekey_map = HashMap::with_capacity(map_len);
        for _ in 0..map_len {
            let exchange_id = r.i64()?;
            let len = r.u32()? as usize;
            temp_rekey_map.insert(exchange_id, r.take(len)?.to_vec());
        }
        Ok(Self {
            id,
            access_hash,
            auth_key,
            admin,
            user_id,
            in_seq_no,
            out_seq_no,
            in_seq_no_x,
            out_seq_no_x,
            layer,
            ttl,
            ttr,
            updated_at_secs,
            mtproto,
            rekeying_state,
            temp_rekey_map,
            sent: VecDeque::new(),
        })
    }
}

/// Default store path for a session: the session path with its extension
/// replaced by `e2e` (`mybot.session` becomes `mybot.e2e`, `mybot` becomes `mybot.e2e`).
pub fn store_path_for_session(session: impl AsRef<Path>) -> PathBuf {
    session.as_ref().with_extension("e2e")
}

/// Temporary state held while waiting for the peer to accept (User A side).
#[derive(Debug)]
pub struct TempChatState {
    /// Secret chat id this exchange belongs to.
    pub id: i32,
    /// The 256-byte private exponent `a` used to compute the shared secret.
    pub private_key_a: [u8; 256],
}

/// In-memory store for established and pending secret chats.
///
/// When a path is set (see [`ChatStore::load`]), `insert` and `remove` flush all
/// established chats to that binary file. Pending exchanges are never persisted.
#[derive(Default)]
pub struct ChatStore {
    chats: HashMap<i32, SecretChatState>,
    pending: HashMap<i32, TempChatState>,
    path: Option<PathBuf>,
}

impl ChatStore {
    /// Create an empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the store from `path`, or start empty if the file doesn't exist yet.
    ///
    /// Fails with `InvalidData` if the file has the wrong magic or version, is
    /// truncated, or does not match its checksum. Later changes are written back
    /// to `path`.
    pub fn load(path: PathBuf) -> io::Result<Self> {
        let chats = match std::fs::read(&path) {
            Ok(data) => Self::decode_store(&data)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            chats,
            pending: HashMap::new(),
            path: Some(path),
        })
    }

    fn decode_store(data: &[u8]) -> io::Result<HashMap<i32, SecretChatState>> {
        if data.len() < STORE_MAGIC.len() + 1 + 4 + CHECKSUM_LEN {
            return Err(invalid("file too short"));
        }
        let (body, checksum) = data.split_at(data.len() - CHECKSUM_LEN);
        if Sha256::digest(body).as_slice() != checksum {
            return Err(invalid("checksum mismatch (file is corrupt)"));
        }
        let mut r = Reader::new(body);
        if r.take(STORE_MAGIC.len())? != STORE_MAGIC {
            return Err(invalid("not an e2e store file"));
        }
        let version = r.u8()?;
        if version != STORE_VERSION {
            return Err(invalid(&format!("unsupported version {version}")));
        }
        let count = r.u32()? as usize;
        let mut chats = HashMap::new();
        for _ in 0..count {
            let chat = SecretChatState::decode(&mut r)?;
            chats.insert(chat.id, chat);
        }
        if r.remaining() != 0 {
            return Err(invalid("trailing bytes"));
        }
        Ok(chats)
    }

    fn encode_store(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.chats.len() * 400);
        out.extend_from_slice(STORE_MAGIC);
        out.push(STORE_VERSION);
        out.extend_from_slice(&(self.chats.len() as u32).to_le_bytes());
        for chat in self.chats.values() {
            chat.encode(&mut out);
        }
        let checksum = Sha256::digest(&out);
        out.extend_from_slice(&checksum);
        out
    }

    /// Write all established chats to the configured path, atomically.
    ///
    /// The data goes to a temporary file next to the target, is synced, and then
    /// renamed over it, so a crash never leaves a half-written store. Does
    /// nothing if no path is set.
    pub fn try_save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let tmp = path.with_extension("e2e.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let result = (|| {
            let mut f = opts.open(&tmp)?;
            f.write_all(&self.encode_store())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Like [`ChatStore::try_save`], but only logs a warning on failure.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            tracing::warn!("e2e: failed to persist secret chats: {e}");
        }
    }

    /// Add an established chat; fails with `ChatAlreadyExists` if the id is taken.
    pub fn insert(&mut self, state: SecretChatState) -> Result<(), E2eError> {
        if self.chats.contains_key(&state.id) {
            return Err(E2eError::ChatAlreadyExists(state.id));
        }
        self.chats.insert(state.id, state);
        self.save();
        Ok(())
    }

    /// Look up an established chat.
    pub fn get(&self, id: i32) -> Result<&SecretChatState, E2eError> {
        self.chats.get(&id).ok_or(E2eError::ChatNotFound(id))
    }

    /// Look up an established chat for modification.
    ///
    /// Changes are not written to disk until [`ChatStore::save`] is called.
    pub fn get_mut(&mut self, id: i32) -> Result<&mut SecretChatState, E2eError> {
        self.chats.get_mut(&id).ok_or(E2eError::ChatNotFound(id))
    }

    /// Forget a chat and any pending exchange for it.
    pub fn remove(&mut self, id: i32) {
        self.chats.remove(&id);
        self.pending.remove(&id);
        self.save();
    }

    /// Remember the private exponent of an outgoing request until the peer accepts.
    pub fn insert_pending(&mut self, state: TempChatState) {
        self.pending.insert(state.id, state);
    }

    /// Take (and remove) the pending exchange for `id`, if any.
    pub fn take_pending(&mut self, id: i32) -> Option<TempChatState> {
        self.pending.remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: i32) -> SecretChatState {
        let mut key = [0u8; 256];
        for (i, b) in key.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(id as u8);
        }
        let mut c = SecretChatState::new(id, 0x1122_3344_5566_7788, key, id % 2 == 0, 4242);
        c.in_seq_no = 5;
        c.out_seq_no = 9;
        c.ttl = 60;
        c.ttr = 77;
        c.layer = 143;
        c.updated_at_secs = 1_700_000_000;
        c
    }

    fn store_at(dir: &tempfile::TempDir) -> (ChatStore, PathBuf) {
        let path = dir.path().join("test.e2e");
        (ChatStore::load(path.clone()).unwrap(), path)
    }

    #[test]
    fn round_trip_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, path) = store_at(&dir);

        let mut a = chat(1);
        a.rekeying_state = RekeyingState::Requested { exchange_id: -7 };
        a.temp_rekey_map.insert(-7, vec![9u8; 256]);
        let mut b = chat(2);
        b.rekeying_state = RekeyingState::Accepted { exchange_id: 55 };
        b.temp_rekey_map.insert(55, vec![1u8; 256]);
        let c = chat(3);
        for s in [a, b, c] {
            store.insert(s).unwrap();
        }

        let loaded = ChatStore::load(path).unwrap();
        for id in 1..=3 {
            let (x, y) = (store.get(id).unwrap(), loaded.get(id).unwrap());
            assert_eq!(x.access_hash, y.access_hash);
            assert_eq!(x.auth_key, y.auth_key);
            assert_eq!(x.admin, y.admin);
            assert_eq!(x.user_id, y.user_id);
            assert_eq!(
                (x.in_seq_no, x.out_seq_no, x.in_seq_no_x, x.out_seq_no_x),
                (y.in_seq_no, y.out_seq_no, y.in_seq_no_x, y.out_seq_no_x)
            );
            assert_eq!((x.layer, x.ttl, x.ttr), (y.layer, y.ttl, y.ttr));
            assert_eq!(x.updated_at_secs, y.updated_at_secs);
            assert_eq!(x.mtproto, y.mtproto);
            assert_eq!(x.temp_rekey_map, y.temp_rekey_map);
            assert_eq!(
                format!("{:?}", x.rekeying_state),
                format!("{:?}", y.rekeying_state)
            );
        }
    }

    #[test]
    fn remove_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, path) = store_at(&dir);
        store.insert(chat(1)).unwrap();
        store.insert(chat(2)).unwrap();
        store.remove(1);
        let loaded = ChatStore::load(path).unwrap();
        assert!(loaded.get(1).is_err());
        assert!(loaded.get(2).is_ok());
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChatStore::load(dir.path().join("nothing.e2e")).unwrap();
        assert!(store.get(1).is_err());
    }

    #[test]
    fn corruption_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, path) = store_at(&dir);
        store.insert(chat(1)).unwrap();

        let mut data = std::fs::read(&path).unwrap();
        data[40] ^= 1;
        std::fs::write(&path, &data).unwrap();
        let err = ChatStore::load(path.clone()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        std::fs::write(&path, &data[..data.len() - 5]).unwrap();
        assert!(ChatStore::load(path.clone()).is_err());

        std::fs::write(&path, b"short").unwrap();
        assert!(ChatStore::load(path).is_err());
    }

    #[test]
    fn wrong_magic_and_version_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, path) = store_at(&dir);
        store.insert(chat(1)).unwrap();
        let good = std::fs::read(&path).unwrap();

        // Re-seal the file with a changed header so only the header check can fail.
        let reseal = |mut body: Vec<u8>| {
            body.truncate(body.len() - CHECKSUM_LEN);
            let sum = Sha256::digest(&body);
            body.extend_from_slice(&sum);
            body
        };
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        std::fs::write(&path, reseal(bad_magic)).unwrap();
        assert!(ChatStore::load(path.clone()).is_err());

        let mut bad_version = good;
        bad_version[STORE_MAGIC.len()] = 99;
        std::fs::write(&path, reseal(bad_version)).unwrap();
        assert!(ChatStore::load(path).is_err());
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = store_at(&dir);
        store.insert(chat(1)).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "unexpected files: {names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn store_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (mut store, path) = store_at(&dir);
        store.insert(chat(1)).unwrap();
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn session_path_maps_to_e2e() {
        assert_eq!(
            store_path_for_session("mybot.session"),
            PathBuf::from("mybot.e2e")
        );
        assert_eq!(store_path_for_session("mybot"), PathBuf::from("mybot.e2e"));
        assert_eq!(
            store_path_for_session("/data/x/hello.session"),
            PathBuf::from("/data/x/hello.e2e")
        );
    }
}
