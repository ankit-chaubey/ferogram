// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram-session: session persistence types and backends for ferogram
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![cfg_attr(docsrs, feature(doc_cfg))]
//! Session persistence types and storage backends for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Most users do not use this crate directly. The `ferogram` crate wires it up
//! automatically when you call `Client::builder().session(...)` or
//! `.session_string(...)`.
//!
//! # What's in here
//!
//! - [`PersistedSession`]: the serializable session struct. Holds the DC table
//!   (one `AuthKey` + salt + time offset per DC), update sequence counters
//!   (PTS, QTS, date, seq), and the peer access-hash cache.
//! - [`SessionBackend`]: the trait all backends implement. A single method:
//!   `save(&PersistedSession)` and `load() -> Option<PersistedSession>`.
//! - [`BinaryFileBackend`]: stores the session as a binary file on disk.
//!   Good for bots and scripts. No extra dependencies.
//! - [`InMemoryBackend`]: keeps everything in memory. Nothing survives process
//!   exit. Used for tests and ephemeral tasks.
//! - [`StringSessionBackend`]: serializes the session to a base64 string.
//!   Useful for serverless environments where you store state in an env var or
//!   database column. Load it with `Client::builder().session_string(s)`.
//! - [`SqliteBackend`]: SQLite-backed storage via rusqlite. Behind the
//!   `sqlite-session` feature flag. Good for local multi-account tooling.
//! - [`LibSqlBackend`]: libSQL / Turso backend. Behind `libsql-session`.
//!   For distributed or edge-hosted session storage.
//!
//! You can also implement `SessionBackend` yourself for Redis, PostgreSQL, or
//! anything else.
//!
//! # Binary format
//!
//! The file backends start with a version byte:
//! - `0x01`: legacy (DC table only, no update state or peer cache).
//! - `0x02`: current (DC table + update state + peer cache).
//!
//! `load()` handles both. `save()` always writes v2.
//!
//! # Example: export and re-import a session
//!
//! ```rust,no_run
//! # async fn example(client: ferogram::Client) -> anyhow::Result<()> {
//! // Export
//! let s = client.export_session_string().await?;
//!
//! // Later, in another process or after a restart:
//! let (client, _) = ferogram::Client::builder()
//!     .api_id(12345)
//!     .api_hash("api_hash")
//!     .session_string(s)
//!     .connect()
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::Path;

#[cfg(feature = "serde")]
mod auth_key_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<[u8; 256]>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(k) => s.serialize_some(k.as_slice()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Option<[u8; 256]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<Vec<u8>> = Option::deserialize(d)?;
        match opt {
            None => Ok(None),
            Some(v) => {
                let arr: [u8; 256] = v
                    .try_into()
                    .map_err(|_| serde::de::Error::custom("auth_key must be exactly 256 bytes"))?;
                Ok(Some(arr))
            }
        }
    }
}

/// Per-DC option flags.
///
/// Stored in the session (v3+) so media DCs survive restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DcFlags(pub u8);

impl DcFlags {
    pub const NONE: DcFlags = DcFlags(0);
    pub const IPV6: DcFlags = DcFlags(1 << 0);
    pub const MEDIA_ONLY: DcFlags = DcFlags(1 << 1);
    pub const TCPO_ONLY: DcFlags = DcFlags(1 << 2);
    pub const CDN: DcFlags = DcFlags(1 << 3);
    pub const STATIC: DcFlags = DcFlags(1 << 4);

    pub fn contains(self, other: DcFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn set(&mut self, flag: DcFlags) {
        self.0 |= flag.0;
    }
}

impl std::ops::BitOr for DcFlags {
    type Output = DcFlags;
    fn bitor(self, rhs: DcFlags) -> DcFlags {
        DcFlags(self.0 | rhs.0)
    }
}

/// One entry in the DC address table.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DcEntry {
    pub dc_id: i32,
    pub addr: String,
    #[cfg_attr(feature = "serde", serde(with = "auth_key_serde"))]
    pub auth_key: Option<[u8; 256]>,
    pub first_salt: i64,
    pub time_offset: i32,
    /// DC capability flags (IPv6, media-only, CDN, ...).
    pub flags: DcFlags,
}

impl DcEntry {
    /// Returns `true` when this entry represents an IPv6 address.
    #[inline]
    pub fn is_ipv6(&self) -> bool {
        self.flags.contains(DcFlags::IPV6)
    }

    /// Parse the stored `"ip:port"` / `"[ipv6]:port"` address into a
    /// [`std::net::SocketAddr`].
    ///
    /// Both formats are valid:
    /// - IPv4: `"149.154.175.53:443"`
    /// - IPv6: `"[2001:b28:f23d:f001::a]:443"`
    pub fn socket_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.addr.parse::<std::net::SocketAddr>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid DC address: {:?}", self.addr),
            )
        })
    }

    /// Construct a `DcEntry` from separate IP string, port, and flags.
    ///
    /// IPv6 addresses are automatically wrapped in brackets so that
    /// `socket_addr()` can round-trip them correctly:
    ///
    /// ```text
    /// DcEntry::from_parts(2, "2001:b28:f23d:f001::a", 443, DcFlags::IPV6)
    /// // addr = "[2001:b28:f23d:f001::a]:443"
    /// ```
    ///
    /// This is the preferred constructor when processing `help.getConfig`
    /// `DcOption` objects from the Telegram API.
    pub fn from_parts(dc_id: i32, ip: &str, port: u16, flags: DcFlags) -> Self {
        // IPv6 addresses contain colons; wrap in brackets for SocketAddr compat.
        let addr = if ip.contains(':') {
            format!("[{ip}]:{port}")
        } else {
            format!("{ip}:{port}")
        };
        Self {
            dc_id,
            addr,
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags,
        }
    }
}

/// Snapshot of the MTProto update-sequence state that we persist so that
/// `catch_up: true` can call `updates.getDifference` with the *pre-shutdown* pts.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UpdatesStateSnap {
    /// Main persistence counter (messages, non-channel updates).
    pub pts: i32,
    /// Secondary counter for secret chats.
    pub qts: i32,
    /// Date of the last received update (Unix timestamp).
    pub date: i32,
    /// Combined-container sequence number.
    pub seq: i32,
    /// Per-channel persistence counters.  `(channel_id, pts)`.
    pub channels: Vec<(i64, i32)>,
}

impl UpdatesStateSnap {
    /// Returns `true` when we have a real state from the server (pts > 0).
    #[inline]
    pub fn is_initialised(&self) -> bool {
        self.pts > 0
    }

    /// Advance (or insert) a per-channel pts value.
    pub fn set_channel_pts(&mut self, channel_id: i64, pts: i32) {
        if let Some(entry) = self.channels.iter_mut().find(|c| c.0 == channel_id) {
            entry.1 = pts;
        } else {
            self.channels.push((channel_id, pts));
        }
    }

    /// Look up the stored pts for a channel, returns 0 if unknown.
    pub fn channel_pts(&self, channel_id: i64) -> i32 {
        self.channels
            .iter()
            .find(|c| c.0 == channel_id)
            .map(|c| c.1)
            .unwrap_or(0)
    }
}

/// A cached access-hash entry so that the peer can be addressed across restarts
/// without re-resolving it from Telegram.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CachedPeer {
    /// Bare Telegram peer ID (always positive).
    pub id: i64,
    /// Access hash bound to the current session.
    /// Always 0 for regular group chats (they need no access_hash).
    pub access_hash: i64,
    /// `true` → channel / supergroup.  `false` → user or regular group.
    pub is_channel: bool,
    /// `true` → regular group chat (Chat::Chat / ChatForbidden).
    /// When true, access_hash is meaningless (groups need no hash).
    pub is_chat: bool,
}

/// A min-user context entry: the user was seen with `min=true` (access_hash
/// not usable directly) so we store the peer+message where they appeared so
/// that `InputPeerUserFromMessage` can be constructed on restart.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CachedMinPeer {
    /// The min user's ID.
    pub user_id: i64,
    /// The channel/chat/user ID of the peer that contained the message.
    pub peer_id: i64,
    /// The message ID within that peer.
    pub msg_id: i32,
}

/// Everything that needs to survive a process restart.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistedSession {
    pub home_dc_id: i32,
    pub dcs: Vec<DcEntry>,
    /// Update counters to enable reliable catch-up after a disconnect.
    pub updates_state: UpdatesStateSnap,
    /// Peer access-hash cache so that the client can reach out to any previously
    /// seen user or channel without re-resolving them.
    pub peers: Vec<CachedPeer>,
    /// Min-user message contexts: users seen with `min=true` that can only be
    /// addressed via `InputPeerUserFromMessage`.
    pub min_peers: Vec<CachedMinPeer>,
}

impl PersistedSession {
    /// Encode the session to raw bytes (v2 binary format).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(512);

        b.push(0x05u8); // version

        b.extend_from_slice(&self.home_dc_id.to_le_bytes());

        b.push(self.dcs.len() as u8);
        for d in &self.dcs {
            b.extend_from_slice(&d.dc_id.to_le_bytes());
            match &d.auth_key {
                Some(k) => {
                    b.push(1);
                    b.extend_from_slice(k);
                }
                None => {
                    b.push(0);
                }
            }
            b.extend_from_slice(&d.first_salt.to_le_bytes());
            b.extend_from_slice(&d.time_offset.to_le_bytes());
            let ab = d.addr.as_bytes();
            b.push(ab.len() as u8);
            b.extend_from_slice(ab);
            b.push(d.flags.0);
        }

        b.extend_from_slice(&self.updates_state.pts.to_le_bytes());
        b.extend_from_slice(&self.updates_state.qts.to_le_bytes());
        b.extend_from_slice(&self.updates_state.date.to_le_bytes());
        b.extend_from_slice(&self.updates_state.seq.to_le_bytes());
        let ch = &self.updates_state.channels;
        b.extend_from_slice(&(ch.len() as u16).to_le_bytes());
        for &(cid, cpts) in ch {
            b.extend_from_slice(&cid.to_le_bytes());
            b.extend_from_slice(&cpts.to_le_bytes());
        }

        // v5 peer type: 0=user, 1=channel, 2=regular-group-chat
        b.extend_from_slice(&(self.peers.len() as u16).to_le_bytes());
        for p in &self.peers {
            b.extend_from_slice(&p.id.to_le_bytes());
            b.extend_from_slice(&p.access_hash.to_le_bytes());
            let peer_type: u8 = if p.is_chat {
                2
            } else if p.is_channel {
                1
            } else {
                0
            };
            b.push(peer_type);
        }

        b.extend_from_slice(&(self.min_peers.len() as u16).to_le_bytes());
        for m in &self.min_peers {
            b.extend_from_slice(&m.user_id.to_le_bytes());
            b.extend_from_slice(&m.peer_id.to_le_bytes());
            b.extend_from_slice(&m.msg_id.to_le_bytes());
        }

        b
    }

    /// Atomically save the session to `path`.
    ///
    /// Writes to `<path>.<seq>.tmp` (unique per call) then renames into place.
    /// A fixed `.tmp` extension causes OS error 2 (ERROR_FILE_NOT_FOUND) on
    /// Windows when two concurrent persist_state calls race: thread A renames
    /// `.tmp` away while thread B's rename finds the source gone.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("{n}.tmp"));
        std::fs::write(&tmp, self.to_bytes())?;
        std::fs::rename(&tmp, path).inspect_err(|_e| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Decode a session from raw bytes (v1 or v2 binary format).
    pub fn from_bytes(buf: &[u8]) -> io::Result<Self> {
        if buf.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidData, "empty session data"));
        }

        let mut p = 0usize;

        macro_rules! r {
            ($n:expr) => {{
                if p + $n > buf.len() {
                    return Err(io::Error::new(ErrorKind::InvalidData, "truncated session"));
                }
                let s = &buf[p..p + $n];
                p += $n;
                s
            }};
        }
        macro_rules! r_i32 {
            () => {
                i32::from_le_bytes(r!(4).try_into().unwrap())
            };
        }
        macro_rules! r_i64 {
            () => {
                i64::from_le_bytes(r!(8).try_into().unwrap())
            };
        }
        macro_rules! r_u8 {
            () => {
                r!(1)[0]
            };
        }
        macro_rules! r_u16 {
            () => {
                u16::from_le_bytes(r!(2).try_into().unwrap())
            };
        }

        let first_byte = r_u8!();

        let (home_dc_id, version) = if first_byte == 0x05 {
            (r_i32!(), 5u8)
        } else if first_byte == 0x04 {
            (r_i32!(), 4u8)
        } else if first_byte == 0x03 {
            (r_i32!(), 3u8)
        } else if first_byte == 0x02 {
            (r_i32!(), 2u8)
        } else {
            let rest = r!(3);
            let mut bytes = [0u8; 4];
            bytes[0] = first_byte;
            bytes[1..4].copy_from_slice(rest);
            (i32::from_le_bytes(bytes), 1u8)
        };

        let dc_count = r_u8!() as usize;
        let mut dcs = Vec::with_capacity(dc_count);
        for _ in 0..dc_count {
            let dc_id = r_i32!();
            let has_key = r_u8!();
            let auth_key = if has_key == 1 {
                let mut k = [0u8; 256];
                k.copy_from_slice(r!(256));
                Some(k)
            } else {
                None
            };
            let first_salt = r_i64!();
            let time_offset = r_i32!();
            let al = r_u8!() as usize;
            let addr = String::from_utf8_lossy(r!(al)).into_owned();
            let flags = if version >= 3 {
                DcFlags(r_u8!())
            } else {
                DcFlags::NONE
            };
            dcs.push(DcEntry {
                dc_id,
                addr,
                auth_key,
                first_salt,
                time_offset,
                flags,
            });
        }

        if version < 2 {
            return Ok(Self {
                home_dc_id,
                dcs,
                updates_state: UpdatesStateSnap::default(),
                peers: Vec::new(),
                min_peers: Vec::new(),
            });
        }

        let pts = r_i32!();
        let qts = r_i32!();
        let date = r_i32!();
        let seq = r_i32!();
        let ch_count = r_u16!() as usize;
        let mut channels = Vec::with_capacity(ch_count);
        for _ in 0..ch_count {
            let cid = r_i64!();
            let cpts = r_i32!();
            channels.push((cid, cpts));
        }

        let peer_count = r_u16!() as usize;
        let mut peers = Vec::with_capacity(peer_count);
        for _ in 0..peer_count {
            let id = r_i64!();
            let access_hash = r_i64!();
            // v5: type byte 0=user, 1=channel, 2=chat; v2-v4: 0=user, 1=channel
            let peer_type = r_u8!();
            let is_channel = peer_type == 1;
            let is_chat = peer_type == 2;
            peers.push(CachedPeer {
                id,
                access_hash,
                is_channel,
                is_chat,
            });
        }

        // v4+: min-user contexts
        let min_peers = if version >= 4 {
            let count = r_u16!() as usize;
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                let user_id = r_i64!();
                let peer_id = r_i64!();
                let msg_id = r_i32!();
                v.push(CachedMinPeer {
                    user_id,
                    peer_id,
                    msg_id,
                });
            }
            v
        } else {
            Vec::new()
        };

        Ok(Self {
            home_dc_id,
            dcs,
            updates_state: UpdatesStateSnap {
                pts,
                qts,
                date,
                seq,
                channels,
            },
            peers,
            min_peers,
        })
    }

    /// Decode a session from a URL-safe base64 string produced by [`to_string`].
    pub fn from_string(s: &str) -> io::Result<Self> {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s.trim())
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        Self::from_bytes(&bytes)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let buf = std::fs::read(path)?;
        Self::from_bytes(&buf)
    }

    // DC address helpers

    /// Find the best DC entry for a given DC ID.
    ///
    /// When `prefer_ipv6` is `true`, returns the IPv6 entry if one is
    /// stored, falling back to IPv4.  When `false`, returns IPv4,
    /// falling back to IPv6.  Returns `None` only when the DC ID is
    /// completely unknown.
    ///
    /// This correctly handles the case where both an IPv4 and an IPv6
    /// `DcEntry` exist for the same `dc_id` (different `flags` bitmask).
    pub fn dc_for(&self, dc_id: i32, prefer_ipv6: bool) -> Option<&DcEntry> {
        let mut candidates = self.dcs.iter().filter(|d| d.dc_id == dc_id).peekable();
        candidates.peek()?;
        // Collect so we can search twice
        let cands: Vec<&DcEntry> = self.dcs.iter().filter(|d| d.dc_id == dc_id).collect();
        // Preferred family first, fall back to whatever is available
        cands
            .iter()
            .copied()
            .find(|d| d.is_ipv6() == prefer_ipv6)
            .or_else(|| cands.first().copied())
    }

    /// Iterate over every stored DC entry for a given DC ID.
    ///
    /// Typically yields one IPv4 and one IPv6 entry per DC ID once
    /// `help.getConfig` has been applied.
    pub fn all_dcs_for(&self, dc_id: i32) -> impl Iterator<Item = &DcEntry> {
        self.dcs.iter().filter(move |d| d.dc_id == dc_id)
    }
}

impl std::fmt::Display for PersistedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use base64::Engine as _;
        f.write_str(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.to_bytes()))
    }
}

/// Bootstrap DC address table (fallback if GetConfig fails).
pub fn default_dc_addresses() -> HashMap<i32, String> {
    [
        (1, "149.154.175.53:443"),
        (2, "149.154.167.51:443"),
        (3, "149.154.175.100:443"),
        (4, "149.154.167.91:443"),
        (5, "91.108.56.130:443"),
    ]
    .into_iter()
    .map(|(id, addr)| (id, addr.to_string()))
    .collect()
}

// session_backend
//
// Pluggable session storage backend.

use std::path::PathBuf;

// Core trait (unchanged)

/// Synchronous snapshot backend: saves and loads the full session at once.
///
/// All built-in backends implement this. Higher-level code should prefer the
/// extension methods below (`update_dc`, `set_home_dc`, `update_state`) which
/// avoid unnecessary full-snapshot writes.
pub trait SessionBackend: Send + Sync {
    fn save(&self, session: &PersistedSession) -> io::Result<()>;
    fn load(&self) -> io::Result<Option<PersistedSession>>;
    fn delete(&self) -> io::Result<()>;

    /// Human-readable name for logging/debug output.
    fn name(&self) -> &str;

    // Granular helpers (default: load → mutate → save)
    //
    // These default implementations are correct but not optimal.
    // Backends that store data in a database (SQLite, libsql, Redis) should
    // override them to issue single-row UPDATE statements instead.

    /// Update a single DC entry without rewriting the entire session.
    ///
    /// Typically called after:
    /// - completing a DH handshake on a new DC (to persist its auth key)
    /// - receiving updated DC addresses from `help.getConfig`
    ///
    fn update_dc(&self, entry: &DcEntry) -> io::Result<()> {
        let mut s = self.load()?.unwrap_or_default();
        // Replace existing entry or append
        if let Some(existing) = s.dcs.iter_mut().find(|d| d.dc_id == entry.dc_id) {
            *existing = entry.clone();
        } else {
            s.dcs.push(entry.clone());
        }
        self.save(&s)
    }

    /// Change the home DC without touching any other session data.
    ///
    /// Called after a successful `*_MIGRATE` redirect: the user's account
    /// now lives on a different DC.
    ///
    fn set_home_dc(&self, dc_id: i32) -> io::Result<()> {
        let mut s = self.load()?.unwrap_or_default();
        s.home_dc_id = dc_id;
        self.save(&s)
    }

    /// Apply a single update-sequence change without a full save/load.
    ///
    ///
    /// `update` is the new partial or full state to merge in.
    fn apply_update_state(&self, update: UpdateStateChange) -> io::Result<()> {
        let mut s = self.load()?.unwrap_or_default();
        update.apply_to(&mut s.updates_state);
        self.save(&s)
    }

    /// Cache a peer access hash without a full session save.
    ///
    /// This is lossy-on-default (full round-trip) but correct.
    /// Override in SQL backends to issue a single `INSERT OR REPLACE`.
    ///
    fn cache_peer(&self, peer: &CachedPeer) -> io::Result<()> {
        let mut s = self.load()?.unwrap_or_default();
        if let Some(existing) = s.peers.iter_mut().find(|p| p.id == peer.id) {
            *existing = peer.clone();
        } else {
            s.peers.push(peer.clone());
        }
        self.save(&s)
    }
}

// UpdateStateChange (mirrors  UpdateState enum)

/// A single update-sequence change, applied via [`SessionBackend::apply_update_state`].
///
///uses:
/// ```text
/// UpdateState::All(updates_state)
/// UpdateState::Primary { pts, date, seq }
/// UpdateState::Secondary { qts }
/// UpdateState::Channel { id, pts }
/// ```
///
/// We map this 1-to-1 to layer's `UpdatesStateSnap`.
#[derive(Debug, Clone)]
pub enum UpdateStateChange {
    /// Replace the entire state snapshot.
    All(UpdatesStateSnap),
    /// Update main sequence counters only (non-channel).
    Primary { pts: i32, date: i32, seq: i32 },
    /// Update the QTS counter (secret chats).
    Secondary { qts: i32 },
    /// Update the PTS for a specific channel.
    Channel { id: i64, pts: i32 },
}

impl UpdateStateChange {
    /// Apply `self` to `snap` in-place.
    pub fn apply_to(&self, snap: &mut UpdatesStateSnap) {
        match self {
            Self::All(new_snap) => *snap = new_snap.clone(),
            Self::Primary { pts, date, seq } => {
                snap.pts = *pts;
                snap.date = *date;
                snap.seq = *seq;
            }
            Self::Secondary { qts } => {
                snap.qts = *qts;
            }
            Self::Channel { id, pts } => {
                // Replace or insert per-channel pts
                if let Some(existing) = snap.channels.iter_mut().find(|c| c.0 == *id) {
                    existing.1 = *pts;
                } else {
                    snap.channels.push((*id, *pts));
                }
            }
        }
    }
}

// BinaryFileBackend

/// Stores the session in a compact binary file (v2 format).
pub struct BinaryFileBackend {
    path: PathBuf,
    /// Serialises concurrent save() calls within the same process so they
    /// don't interleave on the tmp file even if PersistedSession::save uses
    /// unique names (belt-and-suspenders; also prevents torn reads of the
    /// session file from a concurrent load + save).
    write_lock: std::sync::Mutex<()>,
}

impl BinaryFileBackend {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: std::sync::Mutex::new(()),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl SessionBackend for BinaryFileBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> {
        let _guard = self.write_lock.lock().unwrap();
        session.save(&self.path)
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        if !self.path.exists() {
            return Ok(None);
        }
        match PersistedSession::load(&self.path) {
            Ok(s) => Ok(Some(s)),
            Err(e) => {
                let bak = self.path.with_extension("bak");
                tracing::warn!(
                    "[ferogram] Session file {:?} is corrupt ({e}); \
                     renaming to {:?} and starting fresh",
                    self.path,
                    bak
                );
                let _ = std::fs::rename(&self.path, &bak);
                Ok(None)
            }
        }
    }

    fn delete(&self) -> io::Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "binary-file"
    }

    // BinaryFileBackend: the default granular impls (load→mutate→save) are
    // fine since the format is a single compact binary blob. No override needed.
}

// InMemoryBackend

/// Ephemeral in-process session: nothing persisted to disk.
///
/// Override the granular methods to skip the clone overhead of the full
/// snapshot path (we're already in memory, so direct field mutations are
/// cheaper than clone→mutate→replace).
#[derive(Default)]
pub struct InMemoryBackend {
    data: std::sync::Mutex<Option<PersistedSession>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: get a snapshot of the current in-memory state.
    pub fn snapshot(&self) -> Option<PersistedSession> {
        self.data.lock().unwrap().clone()
    }
}

impl SessionBackend for InMemoryBackend {
    fn save(&self, s: &PersistedSession) -> io::Result<()> {
        *self.data.lock().unwrap() = Some(s.clone());
        Ok(())
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        Ok(self.data.lock().unwrap().clone())
    }

    fn delete(&self) -> io::Result<()> {
        *self.data.lock().unwrap() = None;
        Ok(())
    }

    fn name(&self) -> &str {
        "in-memory"
    }

    // Granular overrides: cheaper than load→clone→save

    fn update_dc(&self, entry: &DcEntry) -> io::Result<()> {
        let mut guard = self.data.lock().unwrap();
        let s = guard.get_or_insert_with(PersistedSession::default);
        if let Some(existing) = s.dcs.iter_mut().find(|d| d.dc_id == entry.dc_id) {
            *existing = entry.clone();
        } else {
            s.dcs.push(entry.clone());
        }
        Ok(())
    }

    fn set_home_dc(&self, dc_id: i32) -> io::Result<()> {
        let mut guard = self.data.lock().unwrap();
        let s = guard.get_or_insert_with(PersistedSession::default);
        s.home_dc_id = dc_id;
        Ok(())
    }

    fn apply_update_state(&self, update: UpdateStateChange) -> io::Result<()> {
        let mut guard = self.data.lock().unwrap();
        let s = guard.get_or_insert_with(PersistedSession::default);
        update.apply_to(&mut s.updates_state);
        Ok(())
    }

    fn cache_peer(&self, peer: &CachedPeer) -> io::Result<()> {
        let mut guard = self.data.lock().unwrap();
        let s = guard.get_or_insert_with(PersistedSession::default);
        if let Some(existing) = s.peers.iter_mut().find(|p| p.id == peer.id) {
            *existing = peer.clone();
        } else {
            s.peers.push(peer.clone());
        }
        Ok(())
    }
}

// StringSessionBackend

/// Portable base64 string session backend.
pub struct StringSessionBackend {
    data: std::sync::Mutex<String>,
}

impl StringSessionBackend {
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            data: std::sync::Mutex::new(s.into()),
        }
    }

    pub fn current(&self) -> String {
        self.data.lock().unwrap().clone()
    }
}

impl SessionBackend for StringSessionBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> {
        *self.data.lock().unwrap() = session.to_string();
        Ok(())
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        let s = self.data.lock().unwrap().clone();
        if s.trim().is_empty() {
            return Ok(None);
        }
        PersistedSession::from_string(&s).map(Some)
    }

    fn delete(&self) -> io::Result<()> {
        *self.data.lock().unwrap() = String::new();
        Ok(())
    }

    fn name(&self) -> &str {
        "string-session"
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dc(id: i32) -> DcEntry {
        DcEntry {
            dc_id: id,
            addr: format!("1.2.3.{id}:443"),
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags: DcFlags::NONE,
        }
    }

    fn make_peer(id: i64, hash: i64) -> CachedPeer {
        CachedPeer {
            id,
            access_hash: hash,
            is_channel: false,
            is_chat: false,
        }
    }

    // InMemoryBackend: basic save/load

    #[test]
    fn inmemory_load_returns_none_when_empty() {
        let b = InMemoryBackend::new();
        assert!(b.load().unwrap().is_none());
    }

    #[test]
    fn inmemory_save_then_load_round_trips() {
        let b = InMemoryBackend::new();
        let mut s = PersistedSession::default();
        s.home_dc_id = 3;
        s.dcs.push(make_dc(3));
        b.save(&s).unwrap();

        let loaded = b.load().unwrap().unwrap();
        assert_eq!(loaded.home_dc_id, 3);
        assert_eq!(loaded.dcs.len(), 1);
    }

    #[test]
    fn inmemory_delete_clears_state() {
        let b = InMemoryBackend::new();
        let mut s = PersistedSession::default();
        s.home_dc_id = 2;
        b.save(&s).unwrap();
        b.delete().unwrap();
        assert!(b.load().unwrap().is_none());
    }

    // InMemoryBackend: granular methods

    #[test]
    fn inmemory_update_dc_inserts_new() {
        let b = InMemoryBackend::new();
        b.update_dc(&make_dc(4)).unwrap();
        let s = b.snapshot().unwrap();
        assert_eq!(s.dcs.len(), 1);
        assert_eq!(s.dcs[0].dc_id, 4);
    }

    #[test]
    fn inmemory_update_dc_replaces_existing() {
        let b = InMemoryBackend::new();
        b.update_dc(&make_dc(2)).unwrap();
        let mut updated = make_dc(2);
        updated.addr = "9.9.9.9:443".to_string();
        b.update_dc(&updated).unwrap();

        let s = b.snapshot().unwrap();
        assert_eq!(s.dcs.len(), 1);
        assert_eq!(s.dcs[0].addr, "9.9.9.9:443");
    }

    #[test]
    fn inmemory_set_home_dc() {
        let b = InMemoryBackend::new();
        b.set_home_dc(5).unwrap();
        assert_eq!(b.snapshot().unwrap().home_dc_id, 5);
    }

    #[test]
    fn inmemory_cache_peer_inserts() {
        let b = InMemoryBackend::new();
        b.cache_peer(&make_peer(100, 0xdeadbeef)).unwrap();
        let s = b.snapshot().unwrap();
        assert_eq!(s.peers.len(), 1);
        assert_eq!(s.peers[0].id, 100);
    }

    #[test]
    fn inmemory_cache_peer_updates_existing() {
        let b = InMemoryBackend::new();
        b.cache_peer(&make_peer(100, 111)).unwrap();
        b.cache_peer(&make_peer(100, 222)).unwrap();
        let s = b.snapshot().unwrap();
        assert_eq!(s.peers.len(), 1);
        assert_eq!(s.peers[0].access_hash, 222);
    }

    // UpdateStateChange

    #[test]
    fn update_state_primary() {
        let mut snap = UpdatesStateSnap {
            pts: 0,
            qts: 0,
            date: 0,
            seq: 0,
            channels: vec![],
        };
        UpdateStateChange::Primary {
            pts: 10,
            date: 20,
            seq: 30,
        }
        .apply_to(&mut snap);
        assert_eq!(snap.pts, 10);
        assert_eq!(snap.date, 20);
        assert_eq!(snap.seq, 30);
        assert_eq!(snap.qts, 0); // untouched
    }

    #[test]
    fn update_state_secondary() {
        let mut snap = UpdatesStateSnap {
            pts: 5,
            qts: 0,
            date: 0,
            seq: 0,
            channels: vec![],
        };
        UpdateStateChange::Secondary { qts: 99 }.apply_to(&mut snap);
        assert_eq!(snap.qts, 99);
        assert_eq!(snap.pts, 5); // untouched
    }

    #[test]
    fn update_state_channel_inserts() {
        let mut snap = UpdatesStateSnap {
            pts: 0,
            qts: 0,
            date: 0,
            seq: 0,
            channels: vec![],
        };
        UpdateStateChange::Channel { id: 12345, pts: 42 }.apply_to(&mut snap);
        assert_eq!(snap.channels, vec![(12345, 42)]);
    }

    #[test]
    fn update_state_channel_updates_existing() {
        let mut snap = UpdatesStateSnap {
            pts: 0,
            qts: 0,
            date: 0,
            seq: 0,
            channels: vec![(12345, 10), (67890, 5)],
        };
        UpdateStateChange::Channel { id: 12345, pts: 99 }.apply_to(&mut snap);
        // First channel updated, second untouched
        assert_eq!(snap.channels[0], (12345, 99));
        assert_eq!(snap.channels[1], (67890, 5));
    }

    #[test]
    fn apply_update_state_via_backend() {
        let b = InMemoryBackend::new();
        b.apply_update_state(UpdateStateChange::Primary {
            pts: 7,
            date: 8,
            seq: 9,
        })
        .unwrap();
        let s = b.snapshot().unwrap();
        assert_eq!(s.updates_state.pts, 7);
    }

    // Default impl (BinaryFileBackend trait shape via InMemory smoke)

    #[test]
    fn default_update_dc_via_trait_object() {
        let b: Box<dyn SessionBackend> = Box::new(InMemoryBackend::new());
        b.update_dc(&make_dc(1)).unwrap();
        b.update_dc(&make_dc(2)).unwrap();
        // Can't call snapshot() on trait object, but save/load must be consistent
        let loaded = b.load().unwrap().unwrap();
        assert_eq!(loaded.dcs.len(), 2);
    }

    // IPv6 tests

    fn make_dc_v6(id: i32) -> DcEntry {
        DcEntry {
            dc_id: id,
            addr: format!("[2001:b28:f23d:f00{}::a]:443", id),
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags: DcFlags::IPV6,
        }
    }

    #[test]
    fn dc_entry_from_parts_ipv4() {
        let dc = DcEntry::from_parts(1, "149.154.175.53", 443, DcFlags::NONE);
        assert_eq!(dc.addr, "149.154.175.53:443");
        assert!(!dc.is_ipv6());
        let sa = dc.socket_addr().unwrap();
        assert_eq!(sa.port(), 443);
    }

    #[test]
    fn dc_entry_from_parts_ipv6() {
        let dc = DcEntry::from_parts(2, "2001:b28:f23d:f001::a", 443, DcFlags::IPV6);
        assert_eq!(dc.addr, "[2001:b28:f23d:f001::a]:443");
        assert!(dc.is_ipv6());
        let sa = dc.socket_addr().unwrap();
        assert_eq!(sa.port(), 443);
    }

    #[test]
    fn persisted_session_dc_for_prefers_ipv6() {
        let mut s = PersistedSession::default();
        s.dcs.push(make_dc(2)); // IPv4
        s.dcs.push(make_dc_v6(2)); // IPv6

        let v6 = s.dc_for(2, true).unwrap();
        assert!(v6.is_ipv6());

        let v4 = s.dc_for(2, false).unwrap();
        assert!(!v4.is_ipv6());
    }

    #[test]
    fn persisted_session_dc_for_falls_back_when_only_ipv4() {
        let mut s = PersistedSession::default();
        s.dcs.push(make_dc(3)); // IPv4 only

        // Asking for IPv6 should fall back to IPv4
        let dc = s.dc_for(3, true).unwrap();
        assert!(!dc.is_ipv6());
    }

    #[test]
    fn persisted_session_all_dcs_for_returns_both() {
        let mut s = PersistedSession::default();
        s.dcs.push(make_dc(1));
        s.dcs.push(make_dc_v6(1));
        s.dcs.push(make_dc(2));

        assert_eq!(s.all_dcs_for(1).count(), 2);
        assert_eq!(s.all_dcs_for(2).count(), 1);
        assert_eq!(s.all_dcs_for(5).count(), 0);
    }

    #[test]
    fn inmemory_ipv4_and_ipv6_coexist() {
        let b = InMemoryBackend::new();
        b.update_dc(&make_dc(2)).unwrap(); // IPv4
        b.update_dc(&make_dc_v6(2)).unwrap(); // IPv6

        let s = b.snapshot().unwrap();
        // Both entries must survive they have different flags
        assert_eq!(s.dcs.iter().filter(|d| d.dc_id == 2).count(), 2);
    }

    #[test]
    fn binary_roundtrip_ipv4_and_ipv6() {
        let mut s = PersistedSession::default();
        s.home_dc_id = 2;
        s.dcs.push(make_dc(2));
        s.dcs.push(make_dc_v6(2));

        let bytes = s.to_bytes();
        let loaded = PersistedSession::from_bytes(&bytes).unwrap();
        assert_eq!(loaded.dcs.len(), 2);
        assert_eq!(loaded.dcs.iter().filter(|d| d.is_ipv6()).count(), 1);
        assert_eq!(loaded.dcs.iter().filter(|d| !d.is_ipv6()).count(), 1);
    }
}

// SqliteBackend

/// SQLite-backed session (via `rusqlite`).
///
/// Enabled with the `sqlite-session` Cargo feature.
///
/// # Schema
///
/// Five tables are created on first open (idempotent):
///
/// | Table          | Purpose                                          |
/// |----------------|--------------------------------------------------|
/// | `meta`         | `home_dc_id` and future scalar values            |
/// | `dcs`          | One row per DC (auth key, address, flags, ...)     |
/// | `update_state` | Single-row pts / qts / date / seq                |
/// | `channel_pts`  | Per-channel pts                                  |
/// | `peers`        | Access-hash cache                                |
///
/// # Granular writes
///
/// All [`SessionBackend`] extension methods (`update_dc`, `set_home_dc`,
/// `apply_update_state`, `cache_peer`) issue **single-row SQL statements**
/// instead of the default load-mutate-save round-trip, so they are safe to
/// call frequently (e.g. on every update batch) without performance concerns.
#[cfg(feature = "sqlite-session")]
pub struct SqliteBackend {
    conn: std::sync::Mutex<rusqlite::Connection>,
    label: String,
}

#[cfg(feature = "sqlite-session")]
impl SqliteBackend {
    const SCHEMA: &'static str = "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous  = NORMAL;

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT    PRIMARY KEY,
            value INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS dcs (
            dc_id       INTEGER NOT NULL,
            flags       INTEGER NOT NULL DEFAULT 0,
            addr        TEXT    NOT NULL,
            auth_key    BLOB,
            first_salt  INTEGER NOT NULL DEFAULT 0,
            time_offset INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (dc_id, flags)
        );

        CREATE TABLE IF NOT EXISTS update_state (
            id   INTEGER PRIMARY KEY CHECK (id = 1),
            pts  INTEGER NOT NULL DEFAULT 0,
            qts  INTEGER NOT NULL DEFAULT 0,
            date INTEGER NOT NULL DEFAULT 0,
            seq  INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS channel_pts (
            channel_id INTEGER PRIMARY KEY,
            pts        INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS peers (
            id           INTEGER PRIMARY KEY,
            access_hash  INTEGER NOT NULL,
            is_channel   INTEGER NOT NULL DEFAULT 0
        );
    ";

    /// Open (or create) the SQLite database at `path`.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let label = path.display().to_string();
        let conn = rusqlite::Connection::open(&path).map_err(io::Error::other)?;
        conn.execute_batch(Self::SCHEMA).map_err(io::Error::other)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            label,
        })
    }

    /// Open an in-process SQLite database (useful for tests).
    pub fn in_memory() -> io::Result<Self> {
        let conn = rusqlite::Connection::open_in_memory().map_err(io::Error::other)?;
        conn.execute_batch(Self::SCHEMA).map_err(io::Error::other)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            label: ":memory:".into(),
        })
    }

    fn map_err(e: rusqlite::Error) -> io::Error {
        io::Error::other(e)
    }

    /// Read the full session out of the database.
    fn read_session(conn: &rusqlite::Connection) -> io::Result<PersistedSession> {
        // home_dc_id
        let home_dc_id: i32 = conn
            .query_row("SELECT value FROM meta WHERE key = 'home_dc_id'", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);

        // dcs
        let mut stmt = conn
            .prepare("SELECT dc_id, flags, addr, auth_key, first_salt, time_offset FROM dcs")
            .map_err(Self::map_err)?;
        let dcs = stmt
            .query_map([], |row| {
                let dc_id: i32 = row.get(0)?;
                let flags_raw: u8 = row.get(1)?;
                let addr: String = row.get(2)?;
                let key_blob: Option<Vec<u8>> = row.get(3)?;
                let first_salt: i64 = row.get(4)?;
                let time_offset: i32 = row.get(5)?;
                Ok((dc_id, addr, key_blob, first_salt, time_offset, flags_raw))
            })
            .map_err(Self::map_err)?
            .filter_map(|r| r.ok())
            .map(
                |(dc_id, addr, key_blob, first_salt, time_offset, flags_raw)| {
                    let auth_key = key_blob.and_then(|b| {
                        if b.len() == 256 {
                            let mut k = [0u8; 256];
                            k.copy_from_slice(&b);
                            Some(k)
                        } else {
                            None
                        }
                    });
                    DcEntry {
                        dc_id,
                        addr,
                        auth_key,
                        first_salt,
                        time_offset,
                        flags: DcFlags(flags_raw),
                    }
                },
            )
            .collect();

        // update_state
        let updates_state = conn
            .query_row(
                "SELECT pts, qts, date, seq FROM update_state WHERE id = 1",
                [],
                |r| {
                    Ok(UpdatesStateSnap {
                        pts: r.get(0)?,
                        qts: r.get(1)?,
                        date: r.get(2)?,
                        seq: r.get(3)?,
                        channels: vec![],
                    })
                },
            )
            .unwrap_or_default();

        // channel_pts
        let mut ch_stmt = conn
            .prepare("SELECT channel_id, pts FROM channel_pts")
            .map_err(Self::map_err)?;
        let channels: Vec<(i64, i32)> = ch_stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?)))
            .map_err(Self::map_err)?
            .filter_map(|r| r.ok())
            .collect();

        // peers
        let mut peer_stmt = conn
            .prepare("SELECT id, access_hash, is_channel FROM peers")
            .map_err(Self::map_err)?;
        let peers: Vec<CachedPeer> = peer_stmt
            .query_map([], |r| {
                Ok(CachedPeer {
                    id: r.get(0)?,
                    access_hash: r.get(1)?,
                    is_channel: r.get::<_, i32>(2)? != 0,
                    is_chat: false,
                })
            })
            .map_err(Self::map_err)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(PersistedSession {
            home_dc_id,
            dcs,
            updates_state: UpdatesStateSnap {
                channels,
                ..updates_state
            },
            peers,
            min_peers: Vec::new(),
        })
    }

    /// Write the full session into the database inside a single transaction.
    fn write_session(conn: &rusqlite::Connection, s: &PersistedSession) -> io::Result<()> {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(Self::map_err)?;

        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('home_dc_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![s.home_dc_id],
        )
        .map_err(Self::map_err)?;

        // Replace all DCs
        conn.execute("DELETE FROM dcs", []).map_err(Self::map_err)?;
        for d in &s.dcs {
            conn.execute(
                "INSERT INTO dcs (dc_id, flags, addr, auth_key, first_salt, time_offset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    d.dc_id,
                    d.flags.0,
                    d.addr,
                    d.auth_key.as_ref().map(|k| k.as_ref()),
                    d.first_salt,
                    d.time_offset,
                ],
            )
            .map_err(Self::map_err)?;
        }

        // update_state  pts and qts are monotonic: write_session() must never
        // move them backwards. MAX() ensures a stale snapshot cannot overwrite
        // a fresher value committed by apply_update_state().
        let us = &s.updates_state;
        conn.execute(
            "INSERT INTO update_state (id, pts, qts, date, seq) VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               pts  = MAX(excluded.pts,  update_state.pts),
               qts  = MAX(excluded.qts,  update_state.qts),
               date = excluded.date,
               seq  = excluded.seq",
            rusqlite::params![us.pts, us.qts, us.date, us.seq],
        )
        .map_err(Self::map_err)?;

        conn.execute("DELETE FROM channel_pts", [])
            .map_err(Self::map_err)?;
        for &(cid, cpts) in &us.channels {
            conn.execute(
                "INSERT INTO channel_pts (channel_id, pts) VALUES (?1, ?2)",
                rusqlite::params![cid, cpts],
            )
            .map_err(Self::map_err)?;
        }

        // peers
        conn.execute("DELETE FROM peers", [])
            .map_err(Self::map_err)?;
        for p in &s.peers {
            conn.execute(
                "INSERT INTO peers (id, access_hash, is_channel) VALUES (?1, ?2, ?3)",
                rusqlite::params![p.id, p.access_hash, p.is_channel as i32],
            )
            .map_err(Self::map_err)?;
        }

        conn.execute_batch("COMMIT").map_err(Self::map_err)
    }
}

#[cfg(feature = "sqlite-session")]
impl SessionBackend for SqliteBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        Self::write_session(&conn, session)
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        let conn = self.conn.lock().unwrap();
        // If meta table is empty, no session has been saved yet.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta", [], |r| r.get(0))
            .map_err(Self::map_err)?;
        if count == 0 {
            return Ok(None);
        }
        Self::read_session(&conn).map(Some)
    }

    fn delete(&self) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             DELETE FROM meta;
             DELETE FROM dcs;
             DELETE FROM update_state;
             DELETE FROM channel_pts;
             DELETE FROM peers;
             COMMIT;",
        )
        .map_err(Self::map_err)
    }

    fn name(&self) -> &str {
        &self.label
    }

    // Granular overrides (single-row SQL, no full round-trip)

    fn update_dc(&self, entry: &DcEntry) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO dcs (dc_id, flags, addr, auth_key, first_salt, time_offset)
             VALUES (?1, ?6, ?2, ?3, ?4, ?5)
             ON CONFLICT(dc_id, flags) DO UPDATE SET
               addr        = excluded.addr,
               auth_key    = excluded.auth_key,
               first_salt  = excluded.first_salt,
               time_offset = excluded.time_offset",
            rusqlite::params![
                entry.dc_id,
                entry.addr,
                entry.auth_key.as_ref().map(|k| k.as_ref()),
                entry.first_salt,
                entry.time_offset,
                entry.flags.0,
            ],
        )
        .map(|_| ())
        .map_err(Self::map_err)
    }

    fn set_home_dc(&self, dc_id: i32) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('home_dc_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![dc_id],
        )
        .map(|_| ())
        .map_err(Self::map_err)
    }

    fn apply_update_state(&self, update: UpdateStateChange) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        match update {
            UpdateStateChange::All(snap) => {
                conn.execute(
                    "INSERT INTO update_state (id, pts, qts, date, seq) VALUES (1,?1,?2,?3,?4)
                     ON CONFLICT(id) DO UPDATE SET
                       pts=excluded.pts, qts=excluded.qts,
                       date=excluded.date, seq=excluded.seq",
                    rusqlite::params![snap.pts, snap.qts, snap.date, snap.seq],
                )
                .map_err(Self::map_err)?;
                conn.execute("DELETE FROM channel_pts", [])
                    .map_err(Self::map_err)?;
                for &(cid, cpts) in &snap.channels {
                    conn.execute(
                        "INSERT INTO channel_pts (channel_id, pts) VALUES (?1, ?2)",
                        rusqlite::params![cid, cpts],
                    )
                    .map_err(Self::map_err)?;
                }
                Ok(())
            }
            UpdateStateChange::Primary { pts, date, seq } => conn
                .execute(
                    "INSERT INTO update_state (id, pts, qts, date, seq) VALUES (1,?1,0,?2,?3)
                     ON CONFLICT(id) DO UPDATE SET pts=excluded.pts, date=excluded.date,
                     seq=excluded.seq",
                    rusqlite::params![pts, date, seq],
                )
                .map(|_| ())
                .map_err(Self::map_err),
            UpdateStateChange::Secondary { qts } => conn
                .execute(
                    "INSERT INTO update_state (id, pts, qts, date, seq) VALUES (1,0,?1,0,0)
                     ON CONFLICT(id) DO UPDATE SET qts = excluded.qts",
                    rusqlite::params![qts],
                )
                .map(|_| ())
                .map_err(Self::map_err),
            UpdateStateChange::Channel { id, pts } => conn
                .execute(
                    "INSERT INTO channel_pts (channel_id, pts) VALUES (?1, ?2)
                     ON CONFLICT(channel_id) DO UPDATE SET pts = excluded.pts",
                    rusqlite::params![id, pts],
                )
                .map(|_| ())
                .map_err(Self::map_err),
        }
    }

    fn cache_peer(&self, peer: &CachedPeer) -> io::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO peers (id, access_hash, is_channel) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               access_hash = excluded.access_hash,
               is_channel  = excluded.is_channel",
            rusqlite::params![peer.id, peer.access_hash, peer.is_channel as i32],
        )
        .map(|_| ())
        .map_err(Self::map_err)
    }
}

// LibSqlBackend

/// libSQL-backed session (Turso / embedded replica / in-process).
///
/// Enabled with the `libsql-session` Cargo feature.
///
/// The libSQL API is async; since [`SessionBackend`] methods are sync we
/// block via `tokio::runtime::Handle::current().block_on(...)`.  Always
/// call from inside a Tokio runtime (i.e. the same runtime as the rest of
/// `ferogram`).
///
/// # Connecting
///
/// | Mode              | Constructor                        |
/// |-------------------|------------------------------------|
/// | Local file        | `LibSqlBackend::open_local(path)`  |
/// | In-memory         | `LibSqlBackend::in_memory()`       |
/// | Turso remote      | `LibSqlBackend::open_remote(url, token)` |
/// | Embedded replica  | `LibSqlBackend::open_replica(path, url, token)` |
#[cfg(feature = "libsql-session")]
pub struct LibSqlBackend {
    conn: libsql::Connection,
    label: String,
}

#[cfg(feature = "libsql-session")]
impl LibSqlBackend {
    const SCHEMA: &'static str = "
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT    PRIMARY KEY,
            value INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS dcs (
            dc_id       INTEGER NOT NULL,
            flags       INTEGER NOT NULL DEFAULT 0,
            addr        TEXT    NOT NULL,
            auth_key    BLOB,
            first_salt  INTEGER NOT NULL DEFAULT 0,
            time_offset INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (dc_id, flags)
        );
        CREATE TABLE IF NOT EXISTS update_state (
            id   INTEGER PRIMARY KEY CHECK (id = 1),
            pts  INTEGER NOT NULL DEFAULT 0,
            qts  INTEGER NOT NULL DEFAULT 0,
            date INTEGER NOT NULL DEFAULT 0,
            seq  INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS channel_pts (
            channel_id INTEGER PRIMARY KEY,
            pts        INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS peers (
            id          INTEGER PRIMARY KEY,
            access_hash INTEGER NOT NULL,
            is_channel  INTEGER NOT NULL DEFAULT 0
        );
    ";

    fn block<F, T>(fut: F) -> io::Result<T>
    where
        F: std::future::Future<Output = Result<T, libsql::Error>>,
    {
        tokio::runtime::Handle::current()
            .block_on(fut)
            .map_err(io::Error::other)
    }

    async fn apply_schema(conn: &libsql::Connection) -> Result<(), libsql::Error> {
        conn.execute_batch(Self::SCHEMA).await
    }

    /// Open a local file database.
    pub fn open_local(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let label = path.display().to_string();
        let db = Self::block(async { libsql::Builder::new_local(path).build().await })?;
        let conn = Self::block(async { db.connect() }).map_err(io::Error::other)?;
        Self::block(Self::apply_schema(&conn))?;
        Ok(Self {
            conn: std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
            label,
        })
    }

    /// Open an in-process in-memory database (useful for tests).
    pub fn in_memory() -> io::Result<Self> {
        let db = Self::block(async { libsql::Builder::new_local(":memory:").build().await })?;
        let conn = Self::block(async { db.connect() }).map_err(io::Error::other)?;
        Self::block(Self::apply_schema(&conn))?;
        Ok(Self {
            conn: std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
            label: ":memory:".into(),
        })
    }

    /// Connect to a remote Turso database.
    pub fn open_remote(url: impl Into<String>, auth_token: impl Into<String>) -> io::Result<Self> {
        let url = url.into();
        let label = url.clone();
        let db = Self::block(async {
            libsql::Builder::new_remote(url, auth_token.into())
                .build()
                .await
        })?;
        let conn = Self::block(async { db.connect() }).map_err(io::Error::other)?;
        Self::block(Self::apply_schema(&conn))?;
        Ok(Self {
            conn: std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
            label,
        })
    }

    /// Open an embedded replica (local file + Turso remote sync).
    pub fn open_replica(
        path: impl Into<PathBuf>,
        url: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> io::Result<Self> {
        let path = path.into();
        let label = format!("{} (replica of {})", path.display(), url.into());
        let db = Self::block(async {
            libsql::Builder::new_remote_replica(path, url.into(), auth_token.into())
                .build()
                .await
        })?;
        let conn = Self::block(async { db.connect() }).map_err(io::Error::other)?;
        Self::block(Self::apply_schema(&conn))?;
        Ok(Self {
            conn: std::sync::Arc::new(tokio::sync::Mutex::new(conn)),
            label,
        })
    }

    async fn read_session_async(
        conn: &libsql::Connection,
    ) -> Result<PersistedSession, libsql::Error> {
        use libsql::de;

        // home_dc_id
        let home_dc_id: i32 = conn
            .query("SELECT value FROM meta WHERE key = 'home_dc_id'", ())
            .await?
            .next()
            .await?
            .map(|r| r.get::<i32>(0))
            .transpose()?
            .unwrap_or(0);

        // dcs
        let mut rows = conn
            .query(
                "SELECT dc_id, flags, addr, auth_key, first_salt, time_offset FROM dcs",
                (),
            )
            .await?;
        let mut dcs = Vec::new();
        while let Some(row) = rows.next().await? {
            let dc_id: i32 = row.get(0)?;
            let flags_raw: u8 = row.get::<i64>(1)? as u8;
            let addr: String = row.get(2)?;
            let key_blob: Option<Vec<u8>> = row.get(3)?;
            let first_salt: i64 = row.get(4)?;
            let time_offset: i32 = row.get(5)?;
            let auth_key = match key_blob {
                Some(b) if b.len() == 256 => {
                    let mut k = [0u8; 256];
                    k.copy_from_slice(&b);
                    Some(k)
                }
                Some(b) => {
                    return Err(libsql::Error::Misuse(format!(
                        "auth_key blob must be 256 bytes, got {}",
                        b.len()
                    )));
                }
                None => None,
            };
            dcs.push(DcEntry {
                dc_id,
                addr,
                auth_key,
                first_salt,
                time_offset,
                flags: DcFlags(flags_raw),
            });
        }

        // update_state
        let mut us_row = conn
            .query(
                "SELECT pts, qts, date, seq FROM update_state WHERE id = 1",
                (),
            )
            .await?;
        let updates_state = if let Some(r) = us_row.next().await? {
            UpdatesStateSnap {
                pts: r.get(0)?,
                qts: r.get(1)?,
                date: r.get(2)?,
                seq: r.get(3)?,
                channels: vec![],
            }
        } else {
            UpdatesStateSnap::default()
        };

        // channel_pts
        let mut ch_rows = conn
            .query("SELECT channel_id, pts FROM channel_pts", ())
            .await?;
        let mut channels = Vec::new();
        while let Some(r) = ch_rows.next().await? {
            channels.push((r.get::<i64>(0)?, r.get::<i32>(1)?));
        }

        // peers
        let mut peer_rows = conn
            .query("SELECT id, access_hash, is_channel FROM peers", ())
            .await?;
        let mut peers = Vec::new();
        while let Some(r) = peer_rows.next().await? {
            peers.push(CachedPeer {
                id: r.get(0)?,
                access_hash: r.get(1)?,
                is_channel: r.get::<i32>(2)? != 0,
                is_chat: false,
            });
        }

        Ok(PersistedSession {
            home_dc_id,
            dcs,
            updates_state: UpdatesStateSnap {
                channels,
                ..updates_state
            },
            peers,
            min_peers: Vec::new(),
        })
    }

    async fn write_session_async(
        conn: &libsql::Connection,
        s: &PersistedSession,
    ) -> Result<(), libsql::Error> {
        conn.execute_batch("BEGIN IMMEDIATE").await?;

        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('home_dc_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            libsql::params![s.home_dc_id],
        )
        .await?;

        conn.execute("DELETE FROM dcs", ()).await?;
        for d in &s.dcs {
            conn.execute(
                "INSERT INTO dcs (dc_id, flags, addr, auth_key, first_salt, time_offset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                libsql::params![
                    d.dc_id,
                    d.flags.0 as i64,
                    d.addr.clone(),
                    d.auth_key.map(|k| k.to_vec()),
                    d.first_salt,
                    d.time_offset,
                ],
            )
            .await?;
        }

        let us = &s.updates_state;
        conn.execute(
            "INSERT INTO update_state (id, pts, qts, date, seq) VALUES (1,?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET
               pts  = MAX(excluded.pts,  update_state.pts),
               qts  = MAX(excluded.qts,  update_state.qts),
               date = excluded.date,
               seq  = excluded.seq",
            libsql::params![us.pts, us.qts, us.date, us.seq],
        )
        .await?;

        conn.execute("DELETE FROM channel_pts", ()).await?;
        for &(cid, cpts) in &us.channels {
            conn.execute(
                "INSERT INTO channel_pts (channel_id, pts) VALUES (?1,?2)",
                libsql::params![cid, cpts],
            )
            .await?;
        }

        conn.execute("DELETE FROM peers", ()).await?;
        for p in &s.peers {
            conn.execute(
                "INSERT INTO peers (id, access_hash, is_channel) VALUES (?1,?2,?3)",
                libsql::params![p.id, p.access_hash, p.is_channel as i32],
            )
            .await?;
        }

        conn.execute_batch("COMMIT").await
    }
}

#[cfg(feature = "libsql-session")]
impl SessionBackend for LibSqlBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> {
        let conn = self.conn.clone();
        let session = session.clone();
        Self::block(async move {
            let conn = conn.lock().await;
            Self::write_session_async(&conn, session).await
        })
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        let conn = self.conn.clone();
        let count: i64 = Self::block(async move {
            let conn = conn.lock().await;
            let mut rows = conn.query("SELECT COUNT(*) FROM meta", ()).await?;
            Ok::<i64, libsql::Error>(rows.next().await?.and_then(|r| r.get(0).ok()).unwrap_or(0))
        })?;
        if count == 0 {
            return Ok(None);
        }
        let conn = self.conn.clone();
        Self::block(async move {
            let conn = conn.lock().await;
            Self::read_session_async(&conn).await
        })
        .map(Some)
    }

    fn delete(&self) -> io::Result<()> {
        let conn = self.conn.clone();
        Self::block(async move {
            let conn = conn.lock().await;
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 DELETE FROM meta;
                 DELETE FROM dcs;
                 DELETE FROM update_state;
                 DELETE FROM channel_pts;
                 DELETE FROM peers;
                 COMMIT;",
            )
            .await
        })
    }

    fn name(&self) -> &str {
        &self.label
    }

    // Granular overrides

    fn update_dc(&self, entry: &DcEntry) -> io::Result<()> {
        let conn = self.conn.clone();
        let (dc_id, addr, key, salt, off, flags) = (
            entry.dc_id,
            entry.addr.clone(),
            entry.auth_key.map(|k| k.to_vec()),
            entry.first_salt,
            entry.time_offset,
            entry.flags.0 as i64,
        );
        Self::block(async move {
            let conn = conn.lock().await;
            conn.execute(
                "INSERT INTO dcs (dc_id, flags, addr, auth_key, first_salt, time_offset)
                 VALUES (?1,?6,?2,?3,?4,?5)
                 ON CONFLICT(dc_id, flags) DO UPDATE SET
                   addr=excluded.addr, auth_key=excluded.auth_key,
                   first_salt=excluded.first_salt, time_offset=excluded.time_offset",
                libsql::params![dc_id, addr, key, salt, off, flags],
            )
            .await
            .map(|_| ())
        })
    }

    fn set_home_dc(&self, dc_id: i32) -> io::Result<()> {
        let conn = self.conn.clone();
        Self::block(async move {
            let conn = conn.lock().await;
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('home_dc_id',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                libsql::params![dc_id],
            )
            .await
            .map(|_| ())
        })
    }

    fn apply_update_state(&self, update: UpdateStateChange) -> io::Result<()> {
        let conn = self.conn.clone();
        Self::block(async move {
            let conn = conn.lock().await;
            match update {
                UpdateStateChange::All(snap) => {
                    conn.execute(
                        "INSERT INTO update_state (id,pts,qts,date,seq) VALUES (1,?1,?2,?3,?4)
                         ON CONFLICT(id) DO UPDATE SET pts=excluded.pts,qts=excluded.qts,
                         date=excluded.date,seq=excluded.seq",
                        libsql::params![snap.pts, snap.qts, snap.date, snap.seq],
                    )
                    .await?;
                    conn.execute("DELETE FROM channel_pts", ()).await?;
                    for &(cid, cpts) in &snap.channels {
                        conn.execute(
                            "INSERT INTO channel_pts (channel_id,pts) VALUES (?1,?2)",
                            libsql::params![cid, cpts],
                        )
                        .await?;
                    }
                    Ok(())
                }
                UpdateStateChange::Primary { pts, date, seq } => conn
                    .execute(
                        "INSERT INTO update_state (id,pts,qts,date,seq) VALUES (1,?1,0,?2,?3)
                         ON CONFLICT(id) DO UPDATE SET pts=excluded.pts,date=excluded.date,
                         seq=excluded.seq",
                        libsql::params![pts, date, seq],
                    )
                    .await
                    .map(|_| ()),
                UpdateStateChange::Secondary { qts } => conn
                    .execute(
                        "INSERT INTO update_state (id,pts,qts,date,seq) VALUES (1,0,?1,0,0)
                         ON CONFLICT(id) DO UPDATE SET qts=excluded.qts",
                        libsql::params![qts],
                    )
                    .await
                    .map(|_| ()),
                UpdateStateChange::Channel { id, pts } => conn
                    .execute(
                        "INSERT INTO channel_pts (channel_id,pts) VALUES (?1,?2)
                         ON CONFLICT(channel_id) DO UPDATE SET pts=excluded.pts",
                        libsql::params![id, pts],
                    )
                    .await
                    .map(|_| ()),
            }
        })
    }

    fn cache_peer(&self, peer: &CachedPeer) -> io::Result<()> {
        let conn = self.conn.clone();
        let (id, hash, is_ch) = (peer.id, peer.access_hash, peer.is_channel as i32);
        Self::block(async move {
            let conn = conn.lock().await;
            conn.execute(
                "INSERT INTO peers (id,access_hash,is_channel) VALUES (?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET
                   access_hash=excluded.access_hash,
                   is_channel=excluded.is_channel",
                libsql::params![id, hash, is_ch],
            )
            .await
            .map(|_| ())
        })
    }
}
