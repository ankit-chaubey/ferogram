// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ferogram_tl_types as tl;

use crate::errors::InvocationError;
pub use crate::types::ChannelKind;

impl From<ferogram_session::ChannelKind> for ChannelKind {
    fn from(k: ferogram_session::ChannelKind) -> Self {
        match k {
            ferogram_session::ChannelKind::Broadcast => ChannelKind::Broadcast,
            ferogram_session::ChannelKind::Megagroup => ChannelKind::Megagroup,
            ferogram_session::ChannelKind::Gigagroup => ChannelKind::Gigagroup,
        }
    }
}

impl From<ChannelKind> for ferogram_session::ChannelKind {
    fn from(k: ChannelKind) -> Self {
        match k {
            ChannelKind::Broadcast => ferogram_session::ChannelKind::Broadcast,
            ChannelKind::Megagroup => ferogram_session::ChannelKind::Megagroup,
            ChannelKind::Gigagroup => ferogram_session::ChannelKind::Gigagroup,
        }
    }
}

/// A batch-scoped, read-only map from channel ID to the raw TL chat object.
///
/// Built once per update batch from the `chats` vec and shared (cheaply via
/// `Arc` refcount) across every `IncomingMessage` produced in that batch.
/// When the last message is dropped the map is freed automatically.
pub type PeerMap = Arc<HashMap<i64, tl::enums::Chat>>;

/// Build a `PeerMap` from a slice of TL chat objects.
///
/// Silently ignores `Chat::Empty` and any entry without an ID.
pub fn build_peer_map(chats: &[tl::enums::Chat]) -> Option<PeerMap> {
    if chats.is_empty() {
        return None;
    }
    let mut map = HashMap::with_capacity(chats.len());
    for chat in chats {
        let id = match chat {
            tl::enums::Chat::Channel(c) => c.id,
            tl::enums::Chat::ChannelForbidden(c) => c.id,
            tl::enums::Chat::Chat(c) => c.id,
            tl::enums::Chat::Forbidden(c) => c.id,
            tl::enums::Chat::Community(c) => c.id,
            tl::enums::Chat::CommunityForbidden(c) => c.id,
            tl::enums::Chat::Empty(_) => continue,
        };
        map.insert(id, chat.clone());
    }
    if map.is_empty() {
        None
    } else {
        Some(Arc::new(map))
    }
}

/// Opt-in experimental behaviours that deviate from strict Telegram spec.
///
/// All flags default to `false` (safe / spec-correct).  Enable only what you
/// need after reading the per-field warnings.
///
/// # Example
/// ```rust,no_run
/// use ferogram::{Client, ExperimentalFeatures};
///
/// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (client, _sd) = Client::builder()
///     .api_id(12345)
///     .api_hash("abc")
///     .experimental_features(ExperimentalFeatures {
///         allow_zero_hash: true,   // bot-only; omit for user accounts
///         ..Default::default()
///     })
///     .connect().await?;
/// # Ok(()) }
/// ```
#[derive(Clone, Debug, Default)]
pub struct ExperimentalFeatures {
    /// When no `access_hash` is cached for a user or channel, fall back to
    /// `access_hash = 0` instead of returning [`InvocationError::PeerNotCached`].
    ///
    /// **Bot accounts only.** The Telegram spec explicitly permits `hash = 0`
    /// for bots when only a min-hash is available.  On user accounts this
    /// produces `USER_ID_INVALID` / `CHANNEL_INVALID`.
    pub allow_zero_hash: bool,

    /// When resolving a min-user via `InputPeerUserFromMessage`, if the
    /// containing channel's hash is not cached, proceed with
    /// `channel access_hash = 0` instead of returning
    /// [`InvocationError::PeerNotCached`].
    ///
    /// Almost always wrong.  The inner `InputPeerChannel { access_hash: 0 }`
    /// makes the whole `InputPeerUserFromMessage` invalid and Telegram will
    /// reject it.  Only useful for debugging / testing.
    pub allow_missing_channel_hash: bool,

    /// When `access_hash` is missing for a channel during `getChannelDifference`,
    /// call `channels.getChannels` with `access_hash = 0` to fetch it, cache it,
    /// and retry the diff in the same loop iteration.
    ///
    /// When false (the default), the diff is deferred: the entry stays alive and
    /// the diff retries naturally once the hash arrives via a future update's
    /// entity list.
    ///
    /// **Bot accounts only** for reliable operation. On user accounts
    /// `channels.getChannels { access_hash: 0 }` succeeds only for public channels
    /// and channels you are currently a member of.
    pub auto_resolve_peers: bool,

    /// Enable resumable uploads and downloads.
    ///
    /// When `true`, interrupted transfers save a checkpoint under
    /// `checkpoint_dir` (defaults to `.ferogram-transfers/` next to the
    /// session file). The next call with the same media / file automatically
    /// resumes from where it left off.
    ///
    /// Upload sessions are valid for ~1 hour on Telegram's side; if more time
    /// has passed the upload restarts from scratch automatically.
    ///
    /// Default: `false`.
    pub resumable_transfers: bool,

    /// Directory for transfer checkpoints when `resumable_transfers` is enabled.
    /// If `None`, defaults to `.ferogram-transfers/` next to the session file.
    pub checkpoint_dir: Option<std::path::PathBuf>,

    /// Cache min-user message contexts (`InputPeerUserFromMessage` entries).
    ///
    /// When `false` (the default), users seen with `min=true` are silently
    /// ignored instead of being stored. This keeps session files lean and is
    /// the right choice for gateway / proxy servers that never need to address
    /// min users directly.
    ///
    /// Enable only if your application needs to send messages or make API
    /// calls targeting users that arrive exclusively as min-users (i.e. users
    /// you have never seen with a full access_hash).
    ///
    /// Default: `false`.
    pub cache_min_peers: bool,
}

/// Caches access hashes for users and channels so every API call carries the
/// correct hash without re-resolving peers.
/// A snapshot of what [`PeerCache`] currently holds.
///
/// Returned by [`PeerCache::stats`]. Useful for logging, monitoring, and
/// deciding whether to call [`PeerCache::clear_min_contexts`].
#[derive(Clone, Debug)]
pub struct PeerCacheStats {
    /// Full users with a valid access_hash.
    pub users: usize,
    /// Full channels with a valid access_hash.
    pub channels: usize,
    /// Full communities with a valid access_hash.
    pub communities: usize,
    /// Regular group chats tracked by existence (no hash needed).
    pub chats: usize,
    /// Channels seen with `min=true` (no usable access_hash yet).
    pub min_channels: usize,
    /// Min-user message contexts (`InputPeerUserFromMessage` entries).
    /// Always 0 when `cache_min_peers` is disabled.
    pub min_contexts: usize,
    /// Username reverse-index entries.
    pub usernames: usize,
    /// Phone number reverse-index entries.
    pub phones: usize,
}

/// Discriminates the kind of peer stored in `PeerCache::username_to_peer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerType {
    User,
    Channel,
    Chat,
}

///
/// All fields are `pub` so that `save_session` / `connect` can read/write them
/// directly, and so that advanced callers can inspect the cache.
pub struct PeerCache {
    /// user_id -> access_hash (full users only, min=false)
    pub users: HashMap<i64, i64>,
    /// channel_id -> (access_hash, `Option<ChannelKind>`) (full channels only, min=false)
    pub channels: HashMap<i64, (i64, Option<ChannelKind>)>,
    /// community_id -> access_hash (full communities only, min=false).
    /// Kept separate from `channels`: a Community is a distinct Chat variant
    /// with no `ChannelKind`, even though it resolves to the same
    /// `InputPeer::Channel` shape on the wire.
    pub communities: HashMap<i64, i64>,
    /// Regular group chat IDs (Chat::Chat / ChatForbidden).
    /// Groups need no access_hash; track existence for peer validation.
    pub chats: HashSet<i64>,
    /// Channel IDs seen with min=true. These are real channels but have no
    /// valid access_hash. Stored separately so they are NEVER confused with
    /// regular groups. DO NOT put min channels in `chats`. A min channel must
    /// never become InputPeerChat  - that causes fatal RPC failures.
    pub channels_min: HashSet<i64>,
    /// user_id -> (peer_id, msg_id) for min users seen in a message context.
    /// Min users have an invalid access_hash; they must be referenced via
    /// InputPeerUserFromMessage using the peer and message where they appeared.
    pub min_contexts: HashMap<i64, (i64, i32)>,
    /// Reverse index: lowercase username → (id, PeerType).
    /// Populated by cache_user / cache_chat; always overwritten on update
    /// (usernames can change).
    pub username_to_peer: HashMap<String, (i64, PeerType)>,
    /// Reverse index: E.164 phone → user_id.
    pub phone_to_user: HashMap<String, i64>,
    /// Experimental opt-ins that change error-vs-fallback behaviour.
    pub(crate) experimental: ExperimentalFeatures,
}

impl Default for PeerCache {
    fn default() -> Self {
        Self::new(ExperimentalFeatures::default())
    }
}

impl PeerCache {
    /// Create a new empty cache with the given experimental-feature flags.
    pub fn new(experimental: ExperimentalFeatures) -> Self {
        Self {
            users: HashMap::new(),
            channels: HashMap::new(),
            communities: HashMap::new(),
            chats: HashSet::new(),
            channels_min: HashSet::new(),
            min_contexts: HashMap::new(),
            username_to_peer: HashMap::new(),
            phone_to_user: HashMap::new(),
            experimental,
        }
    }

    pub fn cache_user(&mut self, user: &tl::enums::User) {
        if let tl::enums::User::User(u) = user {
            if u.min {
                // min=true: access_hash is not valid; requires a message context.
            } else if let Some(hash) = u.access_hash {
                // Never overwrite a valid non-zero hash with zero.
                if hash != 0 {
                    self.users.insert(u.id, hash);
                } else {
                    self.users.entry(u.id).or_insert(0);
                }
                // Full user always supersedes any min context.
                self.min_contexts.remove(&u.id);
            }
            // Reverse indices (update even for min users so username lookup works)
            if let Some(ref uname) = u.username {
                self.username_to_peer
                    .insert(uname.to_lowercase(), (u.id, PeerType::User));
            }
            if let Some(ref phone) = u.phone
                && let Some(normalized) = crate::util::normalize_phone(phone)
            {
                self.phone_to_user.insert(normalized, u.id);
            }
        }
    }

    /// Cache a user that arrived in a message context.
    ///
    /// For min users (access_hash is invalid), stores the peer+msg context so
    /// they can later be referenced via `InputPeerUserFromMessage`.
    ///
    /// Uses **latest-wins** semantics: a newer message context replaces the
    /// stored one.  Recent messages are less likely to have been deleted.
    pub fn cache_user_with_context(&mut self, user: &tl::enums::User, peer_id: i64, msg_id: i32) {
        if let tl::enums::User::User(u) = user {
            if u.min {
                // Never downgrade a cached full user to a min context.
                if self.experimental.cache_min_peers && !self.users.contains_key(&u.id) {
                    // Latest-wins: overwrite with the most recent message context.
                    self.min_contexts.insert(u.id, (peer_id, msg_id));
                }
            } else if let Some(hash) = u.access_hash {
                // Never overwrite a non-zero hash with zero.
                if hash != 0 {
                    self.users.insert(u.id, hash);
                } else {
                    self.users.entry(u.id).or_insert(0);
                }
                self.min_contexts.remove(&u.id);
            }
            // Reverse indices
            if let Some(ref uname) = u.username {
                self.username_to_peer
                    .insert(uname.to_lowercase(), (u.id, PeerType::User));
            }
            if let Some(ref phone) = u.phone
                && let Some(normalized) = crate::util::normalize_phone(phone)
            {
                self.phone_to_user.insert(normalized, u.id);
            }
        }
    }

    pub fn cache_chat(&mut self, chat: &tl::enums::Chat) {
        match chat {
            tl::enums::Chat::Channel(c) => {
                let kind = if c.megagroup {
                    Some(ChannelKind::Megagroup)
                } else if c.gigagroup {
                    Some(ChannelKind::Gigagroup)
                } else {
                    Some(ChannelKind::Broadcast)
                };
                if c.min {
                    // min channel: no access_hash available.
                    // Store in channels_min; never put in chats (InputPeerChat fails).
                    if !self.channels.contains_key(&c.id) {
                        self.channels_min.insert(c.id);
                    }
                } else if let Some(hash) = c.access_hash {
                    // Never overwrite a valid non-zero hash with zero.
                    if hash != 0 {
                        self.channels.insert(c.id, (hash, kind));
                    } else {
                        self.channels.entry(c.id).or_insert((0, kind));
                    }
                    // Full channel supersedes any min tracking.
                    self.channels_min.remove(&c.id);
                }
                // Reverse username index for channels (update regardless of min)
                if let Some(ref uname) = c.username {
                    self.username_to_peer
                        .insert(uname.to_lowercase(), (c.id, PeerType::Channel));
                }
            }
            tl::enums::Chat::ChannelForbidden(c) => {
                // ChannelForbidden has no flags; treat as Broadcast kind.
                if c.access_hash != 0 {
                    self.channels
                        .insert(c.id, (c.access_hash, Some(ChannelKind::Broadcast)));
                } else {
                    self.channels
                        .entry(c.id)
                        .or_insert((0, Some(ChannelKind::Broadcast)));
                }
                self.channels_min.remove(&c.id);
            }
            tl::enums::Chat::Chat(c) => {
                // Regular groups need no access_hash; track existence only.
                self.chats.insert(c.id);
            }
            tl::enums::Chat::Forbidden(c) => {
                self.chats.insert(c.id);
            }
            tl::enums::Chat::Community(c) => {
                if c.min {
                    // min community: no usable access_hash yet. Unlike channels,
                    // there is no separate min-tracking set for communities;
                    // a min community that has never had a full hash simply
                    // stays absent from `communities` until one arrives.
                } else if let Some(hash) = c.access_hash {
                    // Never overwrite a valid non-zero hash with zero.
                    if hash != 0 {
                        self.communities.insert(c.id, hash);
                    } else {
                        self.communities.entry(c.id).or_insert(0);
                    }
                }
            }
            tl::enums::Chat::CommunityForbidden(c) => {
                if let Some(hash) = c.access_hash {
                    if hash != 0 {
                        self.communities.insert(c.id, hash);
                    } else {
                        self.communities.entry(c.id).or_insert(0);
                    }
                }
            }
            _ => {}
        }
    }

    /// Look up the cached [`ChannelKind`] for a channel ID.
    ///
    /// Returns `None` when the channel is not in the cache or was loaded from a
    /// pre-v6 session file that predates kind tracking.
    pub fn channel_kind_of(&self, channel_id: i64) -> Option<ChannelKind> {
        self.channels.get(&channel_id).and_then(|&(_, k)| k)
    }

    pub fn cache_users(&mut self, users: &[tl::enums::User]) {
        for u in users {
            self.cache_user(u);
        }
    }

    pub fn cache_chats(&mut self, chats: &[tl::enums::Chat]) {
        for c in chats {
            self.cache_chat(c);
        }
    }

    /// Store an already-resolved `InputPeer`'s access hash into the cache.
    ///
    /// Called when a caller provides a `PeerRef::Input` so that the subsequent
    /// `peer_to_input` lookup succeeds without an RPC.
    pub fn cache_input_peer(&mut self, ip: &tl::enums::InputPeer) {
        match ip {
            tl::enums::InputPeer::User(u) => {
                if u.access_hash != 0 {
                    self.users.insert(u.user_id, u.access_hash);
                } else {
                    self.users.entry(u.user_id).or_insert(0);
                }
                self.min_contexts.remove(&u.user_id);
            }
            tl::enums::InputPeer::Channel(c) => {
                if c.access_hash != 0 {
                    self.channels
                        .entry(c.channel_id)
                        .and_modify(|e| e.0 = c.access_hash)
                        .or_insert((c.access_hash, None));
                } else {
                    self.channels.entry(c.channel_id).or_insert((0, None));
                }
                self.channels_min.remove(&c.channel_id);
            }
            tl::enums::InputPeer::Chat(c) => {
                self.chats.insert(c.chat_id);
            }
            // UserFromMessage: cache the container peer's hash AND record the
            // min_context so peer_to_input() can rebuild InputPeerUserFromMessage.
            tl::enums::InputPeer::UserFromMessage(u) => {
                // Cache the container peer's access hash
                self.cache_input_peer(&u.peer);
                // Extract container peer_id for the min_context entry
                let container_peer_id = match &u.peer {
                    tl::enums::InputPeer::Channel(c) => Some(c.channel_id),
                    tl::enums::InputPeer::Chat(c) => Some(c.chat_id),
                    tl::enums::InputPeer::User(pu) => Some(pu.user_id),
                    tl::enums::InputPeer::PeerSelf => Some(0i64),
                    _ => None,
                };
                if let Some(peer_id) = container_peer_id {
                    // Only set min_context if there is no full hash cached yet.
                    if self.experimental.cache_min_peers && !self.users.contains_key(&u.user_id) {
                        self.min_contexts.insert(u.user_id, (peer_id, u.msg_id));
                    }
                }
            }
            // ChannelFromMessage: cache the container peer hash and channel entry.
            tl::enums::InputPeer::ChannelFromMessage(c) => {
                self.cache_input_peer(&c.peer);
                // The channel itself has no standalone hash here; mark as known
                // via channels_min so we don't lose track of it.
                self.channels_min.insert(c.channel_id);
            }
            tl::enums::InputPeer::Empty | tl::enums::InputPeer::PeerSelf => {}
        }
    }

    /// Remove stale cache entries when Telegram rejects them with
    /// `PEER_ID_INVALID`, `CHANNEL_INVALID`, `USER_ID_INVALID`, or
    /// `CHANNEL_PRIVATE`.  The caller should then retry the operation.
    pub fn invalidate_peer(&mut self, peer: &tl::enums::Peer) {
        match peer {
            tl::enums::Peer::User(u) => {
                self.users.remove(&u.user_id);
                self.min_contexts.remove(&u.user_id);
            }
            tl::enums::Peer::Channel(c) => {
                self.channels.remove(&c.channel_id);
                self.channels_min.remove(&c.channel_id);
            }
            tl::enums::Peer::Chat(_) => {} // basic groups have no hash to invalidate
        }
    }

    pub(crate) fn user_input_peer(
        &self,
        user_id: i64,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
        if user_id == 0 {
            return Ok(tl::enums::InputPeer::PeerSelf);
        }

        // Full hash: best case.
        if let Some(&hash) = self.users.get(&user_id) {
            return Ok(tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id,
                access_hash: hash,
            }));
        }

        // Min user: resolve via the message context where they were seen.
        if let Some(&(peer_id, msg_id)) = self.min_contexts.get(&user_id) {
            // The containing peer can be a channel, a basic group, or a DM user.
            // Build the correct InputPeer variant for each case.
            let container = if let Some(&(hash, _)) = self.channels.get(&peer_id) {
                tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id: peer_id,
                    access_hash: hash,
                })
            } else if self.channels_min.contains(&peer_id) {
                if self.experimental.allow_missing_channel_hash {
                    tracing::warn!(
                        "[ferogram::peer_cache] channel {peer_id} is a min peer \
                         (seen inside message for user {user_id}), using access_hash=0. \
                         This will likely cause CHANNEL_INVALID on user accounts. \
                         Call client.resolve_peer() to get a full access_hash first."
                    );
                    tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                        channel_id: peer_id,
                        access_hash: 0,
                    })
                } else {
                    return Err(InvocationError::PeerNotCached(format!(
                        "min user {user_id} was seen in channel {peer_id}, \
                         but that channel is only known as a min channel (no access_hash). \
                         Resolve the channel first, or enable \
                         ExperimentalFeatures::allow_missing_channel_hash."
                    )));
                }
            } else if self.chats.contains(&peer_id) {
                // Basic group: no access_hash needed.
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: peer_id })
            } else if let Some(&hash) = self.users.get(&peer_id) {
                // DM: min user was seen in a direct message with another user.
                tl::enums::InputPeer::User(tl::types::InputPeerUser {
                    user_id: peer_id,
                    access_hash: hash,
                })
            } else {
                return Err(InvocationError::PeerNotCached(format!(
                    "min user {user_id} was seen in peer {peer_id}, \
                     but that peer is not cached (not a known channel, chat, or user). \
                     Ensure the containing chat flows through the update loop first."
                )));
            };
            return Ok(tl::enums::InputPeer::UserFromMessage(Box::new(
                tl::types::InputPeerUserFromMessage {
                    peer: container,
                    msg_id,
                    user_id,
                },
            )));
        }

        // No hash at all.
        if self.experimental.allow_zero_hash {
            tracing::warn!(
                "[ferogram::peer_cache] no access_hash cached for user {user_id}, using 0. \
                 This is valid for bots but will cause USER_ID_INVALID on user accounts. \
                 Disable ExperimentalFeatures::allow_zero_hash or call resolve_peer() first."
            );
            Ok(tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id,
                access_hash: 0,
            }))
        } else {
            Err(InvocationError::PeerNotCached(format!(
                "no access_hash cached for user {user_id}. \
                 Ensure at least one message from this user flows through the \
                 update loop before using them as a peer, or call \
                 client.resolve_peer() first."
            )))
        }
    }

    fn channel_input_peer(&self, channel_id: i64) -> Result<tl::enums::InputPeer, InvocationError> {
        if let Some(&(hash, _)) = self.channels.get(&channel_id) {
            return Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id,
                access_hash: hash,
            }));
        }

        if self.experimental.allow_zero_hash {
            tracing::warn!(
                "[ferogram::peer_cache] no access_hash cached for channel {channel_id}, using 0. \
                 This is valid for bots but will cause CHANNEL_INVALID on user accounts. \
                 Disable ExperimentalFeatures::allow_zero_hash or call resolve_peer() first."
            );
            Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id,
                access_hash: 0,
            }))
        } else {
            Err(InvocationError::PeerNotCached(format!(
                "no access_hash cached for channel {channel_id}. \
                 Ensure the channel flows through the update loop before using \
                 it as a peer, or call client.resolve_peer() first."
            )))
        }
    }

    /// Resolve a cached community ID to an `InputPeer`.
    ///
    /// A Community has no dedicated `InputPeer` variant on the wire, it is
    /// addressed exactly like a channel (`InputPeer::Channel`), just tracked
    /// in a separate cache bucket so it is never mistaken for one. Called
    /// from [`PeerCache::peer_to_input`] as the fallback for a `Peer::Channel`
    /// whose ID isn't in `channels` - a bare numeric ID always decodes to
    /// `Peer::Channel` since there is no `Peer::Community`, so a community
    /// looks exactly like an uncached channel until this check runs.
    pub(crate) fn community_input_peer(
        &self,
        community_id: i64,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
        if let Some(&hash) = self.communities.get(&community_id) {
            return Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: community_id,
                access_hash: hash,
            }));
        }

        if self.experimental.allow_zero_hash {
            tracing::warn!(
                "[ferogram::peer_cache] no access_hash cached for community {community_id}, using 0. \
                 This is valid for bots but will cause CHANNEL_INVALID on user accounts. \
                 Disable ExperimentalFeatures::allow_zero_hash or call resolve_peer() first."
            );
            Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: community_id,
                access_hash: 0,
            }))
        } else {
            Err(InvocationError::PeerNotCached(format!(
                "no access_hash cached for community {community_id}. \
                 Ensure the community flows through the update loop before using \
                 it as a peer, or call client.resolve_peer() first."
            )))
        }
    }

    /// Drop all cached min-user contexts.
    ///
    /// Safe to call at any time. After this, any min user that has no full
    /// access_hash returns [`InvocationError::PeerNotCached`] until they
    /// appear again in an update (and `cache_min_peers` is enabled) or until
    /// `resolve_peer()` fetches their full hash.
    ///
    /// Call this periodically on gateway servers to bound memory growth from
    /// min-user entries that accumulate over long uptimes.
    pub fn clear_min_contexts(&mut self) {
        self.min_contexts.clear();
    }

    /// A point-in-time snapshot of entry counts in this cache.
    ///
    /// Cheap to call, no allocation beyond the returned struct itself.
    pub fn stats(&self) -> PeerCacheStats {
        PeerCacheStats {
            users: self.users.len(),
            channels: self.channels.len(),
            communities: self.communities.len(),
            chats: self.chats.len(),
            min_channels: self.channels_min.len(),
            min_contexts: self.min_contexts.len(),
            usernames: self.username_to_peer.len(),
            phones: self.phone_to_user.len(),
        }
    }

    pub fn peer_to_input(
        &self,
        peer: &tl::enums::Peer,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
        match peer {
            tl::enums::Peer::User(u) => self.user_input_peer(u.user_id),
            tl::enums::Peer::Chat(c) => Ok(tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                chat_id: c.chat_id,
            })),
            // `Peer::Channel` is what a bare numeric ID always decodes to
            // (there is no `Peer::Community`), so a community and a channel
            // are indistinguishable at this point. Check the community
            // bucket before falling into `channel_input_peer`'s zero-hash
            // fallback, otherwise a cached community hash would never be
            // reached on accounts with `allow_zero_hash` enabled.
            tl::enums::Peer::Channel(c) => {
                if self.channels.contains_key(&c.channel_id) {
                    self.channel_input_peer(c.channel_id)
                } else if self.communities.contains_key(&c.channel_id) {
                    self.community_input_peer(c.channel_id)
                } else {
                    self.channel_input_peer(c.channel_id)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn community(id: i64, min: bool, access_hash: Option<i64>) -> tl::enums::Chat {
        tl::enums::Chat::Community(tl::types::Community {
            creator: false,
            left: false,
            min,
            collapsed_in_dialogs: false,
            id,
            access_hash,
            title: format!("community-{id}"),
            photo: tl::enums::ChatPhoto::Empty,
            date: 0,
            admin_rights: None,
            default_banned_rights: None,
        })
    }

    fn community_forbidden(id: i64, access_hash: Option<i64>) -> tl::enums::Chat {
        tl::enums::Chat::CommunityForbidden(tl::types::CommunityForbidden {
            id,
            access_hash,
            title: format!("community-{id}"),
        })
    }

    #[test]
    fn cache_chat_stores_full_community() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community(100, false, Some(0xabc)));
        assert_eq!(cache.communities.get(&100), Some(&0xabc));
    }

    #[test]
    fn cache_chat_ignores_min_community() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community(101, true, None));
        assert!(!cache.communities.contains_key(&101));
    }

    #[test]
    fn cache_chat_never_downgrades_community_hash_to_zero() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community(102, false, Some(0xdead)));
        cache.cache_chat(&community(102, false, Some(0)));
        assert_eq!(cache.communities.get(&102), Some(&0xdead));
    }

    #[test]
    fn cache_chat_stores_community_forbidden() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community_forbidden(103, Some(0x111)));
        assert_eq!(cache.communities.get(&103), Some(&0x111));
    }

    #[test]
    fn community_input_peer_resolves_cached_hash() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community(200, false, Some(0x999)));

        let input = cache.community_input_peer(200).unwrap();
        match input {
            tl::enums::InputPeer::Channel(c) => {
                assert_eq!(c.channel_id, 200);
                assert_eq!(c.access_hash, 0x999);
            }
            other => panic!("expected InputPeer::Channel, got {other:?}"),
        }
    }

    #[test]
    fn community_input_peer_errors_when_uncached() {
        let cache = PeerCache::default();
        assert!(matches!(
            cache.community_input_peer(999),
            Err(InvocationError::PeerNotCached(_))
        ));
    }

    #[test]
    fn community_input_peer_falls_back_to_zero_hash_when_enabled() {
        let cache = PeerCache::new(ExperimentalFeatures {
            allow_zero_hash: true,
            ..Default::default()
        });
        let input = cache.community_input_peer(321).unwrap();
        match input {
            tl::enums::InputPeer::Channel(c) => {
                assert_eq!(c.channel_id, 321);
                assert_eq!(c.access_hash, 0);
            }
            other => panic!("expected InputPeer::Channel, got {other:?}"),
        }
    }

    #[test]
    fn stats_reports_community_count() {
        let mut cache = PeerCache::default();
        cache.cache_chat(&community(400, false, Some(0x1)));
        cache.cache_chat(&community(401, false, Some(0x2)));
        assert_eq!(cache.stats().communities, 2);
    }
}
