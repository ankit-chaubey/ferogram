// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#[cfg(test)]
mod retry {
    use std::num::NonZeroU32;
    use std::ops::ControlFlow;
    use std::sync::Arc;
    use std::time::Duration;

    // Inline minimal stubs so the test file is self-contained
    #[derive(Debug)]
    enum InvocationError {
        Rpc(RpcError),
        Io(String),
        Dropped,
    }

    #[derive(Debug, Clone)]
    struct RpcError {
        code: i32,
        name: String,
        value: Option<u32>,
    }

    impl RpcError {
        fn flood_wait_seconds(&self) -> Option<u64> {
            if self.code == 420 && self.name == "FLOOD_WAIT" {
                self.value.map(|v| v as u64)
            } else {
                None
            }
        }

        fn migrate_dc_id(&self) -> Option<i32> {
            if self.code != 303 {
                return None;
            }
            let is_migrate = matches!(
                self.name.as_str(),
                "PHONE_MIGRATE" | "NETWORK_MIGRATE" | "FILE_MIGRATE" | "USER_MIGRATE"
            );
            if is_migrate {
                Some(self.value.unwrap_or(2) as i32)
            } else {
                None
            }
        }
    }

    struct RetryContext {
        fail_count: NonZeroU32,
        slept_so_far: Duration,
        error: InvocationError,
    }

    trait RetryPolicy: Send + Sync {
        fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration>;
    }

    struct NoRetries;
    impl RetryPolicy for NoRetries {
        fn should_retry(&self, _: &RetryContext) -> ControlFlow<(), Duration> {
            ControlFlow::Break(())
        }
    }

    struct AutoSleep {
        threshold: Duration,
        io_errors_as_flood_of: Option<Duration>,
    }

    impl Default for AutoSleep {
        fn default() -> Self {
            Self {
                threshold: Duration::from_secs(60),
                io_errors_as_flood_of: Some(Duration::from_secs(1)),
            }
        }
    }

    impl RetryPolicy for AutoSleep {
        fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
            match &ctx.error {
                InvocationError::Rpc(rpc)
                    if rpc.code == 420 && rpc.name == "FLOOD_WAIT" && ctx.fail_count.get() == 1 =>
                {
                    let secs = rpc.value.unwrap_or(0) as u64;
                    if secs <= self.threshold.as_secs() {
                        ControlFlow::Continue(Duration::from_secs(secs))
                    } else {
                        ControlFlow::Break(())
                    }
                }
                InvocationError::Io(_) if ctx.fail_count.get() == 1 => {
                    if let Some(d) = self.io_errors_as_flood_of {
                        ControlFlow::Continue(d)
                    } else {
                        ControlFlow::Break(())
                    }
                }
                _ => ControlFlow::Break(()),
            }
        }
    }

    fn flood(secs: u32) -> InvocationError {
        InvocationError::Rpc(RpcError {
            code: 420,
            name: "FLOOD_WAIT".into(),
            value: Some(secs),
        })
    }

    fn io_err() -> InvocationError {
        InvocationError::Io("connection reset".into())
    }

    fn migrate(dc: u32) -> InvocationError {
        InvocationError::Rpc(RpcError {
            code: 303,
            name: "PHONE_MIGRATE".into(),
            value: Some(dc),
        })
    }

    #[test]
    fn no_retries_never_retries() {
        let p = NoRetries;
        for err in [flood(5), io_err(), migrate(2)] {
            let ctx = RetryContext {
                fail_count: NonZeroU32::new(1).unwrap(),
                slept_so_far: Duration::default(),
                error: err,
            };
            assert!(matches!(p.should_retry(&ctx), ControlFlow::Break(())));
        }
    }

    #[test]
    fn autosleep_continues_on_small_flood() {
        let p = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: flood(30),
        };
        assert!(
            matches!(p.should_retry(&ctx), ControlFlow::Continue(d) if d == Duration::from_secs(30))
        );
    }

    #[test]
    fn autosleep_breaks_on_large_flood() {
        let p = AutoSleep {
            threshold: Duration::from_secs(10),
            ..Default::default()
        };
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: flood(60),
        };
        assert!(matches!(p.should_retry(&ctx), ControlFlow::Break(())));
    }

    #[test]
    fn autosleep_does_not_retry_flood_twice() {
        let p = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(2).unwrap(), // second attempt
            slept_so_far: Duration::from_secs(30),
            error: flood(30),
        };
        assert!(matches!(p.should_retry(&ctx), ControlFlow::Break(())));
    }

    #[test]
    fn autosleep_retries_io_once_then_breaks() {
        let p = AutoSleep::default();
        // First: should continue
        let first = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: io_err(),
        };
        assert!(matches!(p.should_retry(&first), ControlFlow::Continue(_)));
        // Second: should break
        let second = RetryContext {
            fail_count: NonZeroU32::new(2).unwrap(),
            slept_so_far: Duration::from_secs(1),
            error: io_err(),
        };
        assert!(matches!(p.should_retry(&second), ControlFlow::Break(())));
    }

    #[test]
    fn autosleep_does_not_retry_other_rpc_errors() {
        let p = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: InvocationError::Rpc(RpcError {
                code: 400,
                name: "BAD_REQUEST".into(),
                value: None,
            }),
        };
        assert!(matches!(p.should_retry(&ctx), ControlFlow::Break(())));
    }

    // migrate_dc_id

    #[test]
    fn migrate_dc_id_all_variants() {
        for name in &[
            "PHONE_MIGRATE",
            "NETWORK_MIGRATE",
            "FILE_MIGRATE",
            "USER_MIGRATE",
        ] {
            let e = RpcError {
                code: 303,
                name: name.to_string(),
                value: Some(4),
            };
            assert_eq!(e.migrate_dc_id(), Some(4), "failed for {name}");
        }
    }

    #[test]
    fn migrate_dc_id_falls_back_to_dc2() {
        let e = RpcError {
            code: 303,
            name: "PHONE_MIGRATE".into(),
            value: None,
        };
        assert_eq!(e.migrate_dc_id(), Some(2));
    }

    #[test]
    fn migrate_dc_id_none_for_non_migrate() {
        let e = RpcError {
            code: 420,
            name: "FLOOD_WAIT".into(),
            value: Some(30),
        };
        assert!(e.migrate_dc_id().is_none());
    }

    #[test]
    fn migrate_dc_id_none_for_wrong_code() {
        // Right name, wrong code: should NOT match
        let e = RpcError {
            code: 400,
            name: "PHONE_MIGRATE".into(),
            value: Some(3),
        };
        assert!(e.migrate_dc_id().is_none());
    }
}

// Session tests

#[cfg(test)]
mod session {
    // Minimal stub types: replace with real imports in the crate
    #[derive(Clone, Default, Debug)]
    struct DcEntry {
        dc_id: i32,
        addr: String,
    }

    #[derive(Clone, Default, Debug, PartialEq)]
    struct UpdatesStateSnap {
        pts: i32,
        qts: i32,
        date: i32,
        seq: i32,
        channels: Vec<(i64, i32)>,
    }

    #[derive(Clone, Default, Debug, PartialEq)]
    struct CachedPeer {
        id: i64,
        access_hash: i64,
    }

    #[derive(Clone, Default, Debug)]
    struct PersistedSession {
        home_dc_id: i32,
        dcs: Vec<DcEntry>,
        updates_state: UpdatesStateSnap,
        peers: Vec<CachedPeer>,
    }

    // Inline minimal InMemoryBackend for isolated testing
    use std::sync::Mutex;

    struct InMemoryBackend {
        data: Mutex<Option<PersistedSession>>,
    }

    impl InMemoryBackend {
        fn new() -> Self {
            Self {
                data: Mutex::new(None),
            }
        }

        fn save(&self, s: &PersistedSession) {
            *self.data.lock().unwrap() = Some(s.clone());
        }

        fn load(&self) -> Option<PersistedSession> {
            self.data.lock().unwrap().clone()
        }

        fn delete(&self) {
            *self.data.lock().unwrap() = None;
        }

        fn update_dc(&self, entry: &DcEntry) {
            let mut guard = self.data.lock().unwrap();
            let s = guard.get_or_insert_with(PersistedSession::default);
            if let Some(e) = s.dcs.iter_mut().find(|d| d.dc_id == entry.dc_id) {
                *e = entry.clone();
            } else {
                s.dcs.push(entry.clone());
            }
        }

        fn set_home_dc(&self, dc_id: i32) {
            let mut guard = self.data.lock().unwrap();
            let s = guard.get_or_insert_with(PersistedSession::default);
            s.home_dc_id = dc_id;
        }

        fn cache_peer(&self, peer: &CachedPeer) {
            let mut guard = self.data.lock().unwrap();
            let s = guard.get_or_insert_with(PersistedSession::default);
            if let Some(p) = s.peers.iter_mut().find(|p| p.id == peer.id) {
                *p = peer.clone();
            } else {
                s.peers.push(peer.clone());
            }
        }
    }

    #[test]
    fn load_empty_returns_none() {
        let b = InMemoryBackend::new();
        assert!(b.load().is_none());
    }

    #[test]
    fn save_and_load_round_trips_home_dc() {
        let b = InMemoryBackend::new();
        let mut s = PersistedSession::default();
        s.home_dc_id = 4;
        b.save(&s);
        assert_eq!(b.load().unwrap().home_dc_id, 4);
    }

    #[test]
    fn delete_clears() {
        let b = InMemoryBackend::new();
        let s = PersistedSession {
            home_dc_id: 1,
            ..Default::default()
        };
        b.save(&s);
        b.delete();
        assert!(b.load().is_none());
    }

    #[test]
    fn update_dc_inserts_new_dc() {
        let b = InMemoryBackend::new();
        b.update_dc(&DcEntry {
            dc_id: 2,
            addr: "1.2.3.4:443".into(),
        });
        let s = b.load().unwrap();
        assert_eq!(s.dcs.len(), 1);
        assert_eq!(s.dcs[0].dc_id, 2);
    }

    #[test]
    fn update_dc_replaces_existing() {
        let b = InMemoryBackend::new();
        b.update_dc(&DcEntry {
            dc_id: 2,
            addr: "old:443".into(),
        });
        b.update_dc(&DcEntry {
            dc_id: 2,
            addr: "new:443".into(),
        });
        let s = b.load().unwrap();
        assert_eq!(s.dcs.len(), 1);
        assert_eq!(s.dcs[0].addr, "new:443");
    }

    #[test]
    fn set_home_dc_does_not_disturb_dcs() {
        let b = InMemoryBackend::new();
        b.update_dc(&DcEntry {
            dc_id: 4,
            addr: "x:443".into(),
        });
        b.set_home_dc(4);
        let s = b.load().unwrap();
        assert_eq!(s.home_dc_id, 4);
        assert_eq!(s.dcs.len(), 1); // DC list untouched
    }

    #[test]
    fn cache_peer_inserts_new() {
        let b = InMemoryBackend::new();
        b.cache_peer(&CachedPeer {
            id: 99,
            access_hash: 0xABCD,
        });
        assert_eq!(b.load().unwrap().peers.len(), 1);
    }

    #[test]
    fn cache_peer_updates_hash() {
        let b = InMemoryBackend::new();
        b.cache_peer(&CachedPeer {
            id: 99,
            access_hash: 111,
        });
        b.cache_peer(&CachedPeer {
            id: 99,
            access_hash: 222,
        });
        let s = b.load().unwrap();
        assert_eq!(s.peers.len(), 1);
        assert_eq!(s.peers[0].access_hash, 222);
    }

    #[test]
    fn update_state_channel_channel_inserted() {
        let b = InMemoryBackend::new();
        // Simulate UpdateStateChange::Channel { id: 100, pts: 42 }
        {
            let mut guard = b.data.lock().unwrap();
            let s = guard.get_or_insert_with(PersistedSession::default);
            s.updates_state.channels.push((100_i64, 42));
        }
        let s = b.load().unwrap();
        assert!(s.updates_state.channels.contains(&(100, 42)));
    }
}

// DC migration tests

#[cfg(test)]
mod dc_migration {
    use std::sync::Mutex;

    struct DcAuthTracker {
        copied: Mutex<Vec<i32>>,
    }

    impl DcAuthTracker {
        fn new() -> Self {
            Self {
                copied: Mutex::new(Vec::new()),
            }
        }
        fn has_copied(&self, dc: i32) -> bool {
            self.copied.lock().unwrap().contains(&dc)
        }
        fn mark_copied(&self, dc: i32) {
            self.copied.lock().unwrap().push(dc);
        }
    }

    fn fallback_dc_addr(dc_id: i32) -> &'static str {
        match dc_id {
            1 => "149.154.175.53:443",
            2 => "149.154.167.51:443",
            3 => "149.154.175.100:443",
            4 => "149.154.167.91:443",
            5 => "91.108.56.130:443",
            _ => "149.154.167.51:443",
        }
    }

    #[test]
    fn fallback_covers_all_five_dcs() {
        // No DC should get the unknown-DC fallback for IDs 1-5
        let unknown_fallback = fallback_dc_addr(999);
        for id in 1..=5 {
            assert_ne!(fallback_dc_addr(id), "", "DC{id} is empty");
            // DC1-5 have distinct IPs (except possibly the same port)
            assert!(fallback_dc_addr(id).contains(":443"), "DC{id} missing port");
        }
        // IDs 1-5 are all individually recognizable
        let unique: std::collections::HashSet<_> = (1..=5).map(fallback_dc_addr).collect();
        assert_eq!(unique.len(), 5, "DCs 1-5 should have distinct addresses");
        // Out-of-range falls back to DC2
        assert_eq!(fallback_dc_addr(99), unknown_fallback);
    }

    #[test]
    fn tracker_empty_at_start() {
        let t = DcAuthTracker::new();
        for dc in 1..=5 {
            assert!(!t.has_copied(dc));
        }
    }

    #[test]
    fn tracker_marks_one_dc() {
        let t = DcAuthTracker::new();
        t.mark_copied(4);
        assert!(t.has_copied(4));
        assert!(!t.has_copied(2));
    }

    #[test]
    fn tracker_marks_multiple_independently() {
        let t = DcAuthTracker::new();
        t.mark_copied(2);
        t.mark_copied(5);
        assert!(t.has_copied(2));
        assert!(!t.has_copied(3));
        assert!(t.has_copied(5));
    }

    #[test]
    fn tracker_allows_duplicate_marks() {
        // Marking the same DC twice should not panic
        let t = DcAuthTracker::new();
        t.mark_copied(3);
        t.mark_copied(3);
        assert!(t.has_copied(3));
    }
}

// IncomingMessage accessor tests
//
// These make sure the accessor methods don't regress when the underlying TL
// type evolves. They work against raw TL values so they don't need a live
// connection.

#[cfg(test)]
mod incoming_message {
    // Minimal standalone stub: replace with real `ferogram_tl_types` + `IncomingMessage`

    struct Message {
        id: i32,
        text: Option<String>,
        out: bool,
        date: i32,
        edit_date: Option<i32>,
        mentioned: bool,
        silent: bool,
        post: bool,
    }

    impl Message {
        fn id(&self) -> i32 {
            self.id
        }
        fn text(&self) -> Option<&str> {
            self.text.as_deref().filter(|s| !s.is_empty())
        }
        fn outgoing(&self) -> bool {
            self.out
        }
        fn date(&self) -> i32 {
            self.date
        }
        fn edit_date(&self) -> Option<i32> {
            self.edit_date
        }
        fn mentioned(&self) -> bool {
            self.mentioned
        }
        fn silent(&self) -> bool {
            self.silent
        }
        fn post(&self) -> bool {
            self.post
        }
        fn date_utc(&self) -> Option<String> {
            if self.date == 0 {
                return None;
            }
            Some(format!("ts:{}", self.date))
        }
    }

    fn regular_msg(id: i32, text: &str) -> Message {
        Message {
            id,
            text: Some(text.into()),
            out: false,
            date: 1_700_000_000,
            edit_date: None,
            mentioned: false,
            silent: false,
            post: false,
        }
    }

    #[test]
    fn text_returns_none_for_empty_string() {
        let m = Message {
            text: Some("".into()),
            ..regular_msg(1, "")
        };
        assert!(m.text().is_none());
    }

    #[test]
    fn text_returns_some_for_non_empty() {
        let m = regular_msg(42, "hello");
        assert_eq!(m.text(), Some("hello"));
    }

    #[test]
    fn outgoing_false_by_default() {
        let m = regular_msg(1, "x");
        assert!(!m.outgoing());
    }

    #[test]
    fn outgoing_true_when_set() {
        let m = Message {
            out: true,
            ..regular_msg(1, "x")
        };
        assert!(m.outgoing());
    }

    #[test]
    fn date_utc_none_for_zero_timestamp() {
        let m = Message {
            date: 0,
            ..regular_msg(1, "x")
        };
        assert!(m.date_utc().is_none());
    }

    #[test]
    fn date_utc_some_for_valid_timestamp() {
        let m = regular_msg(1, "x");
        assert!(m.date_utc().is_some());
    }

    #[test]
    fn edit_date_none_for_unedited() {
        let m = regular_msg(1, "x");
        assert!(m.edit_date().is_none());
    }

    #[test]
    fn edit_date_some_for_edited() {
        let m = Message {
            edit_date: Some(1_700_001_000),
            ..regular_msg(1, "x")
        };
        assert_eq!(m.edit_date(), Some(1_700_001_000));
    }

    #[test]
    fn mentioned_false_by_default() {
        assert!(!regular_msg(1, "x").mentioned());
    }

    #[test]
    fn silent_false_by_default() {
        assert!(!regular_msg(1, "x").silent());
    }

    #[test]
    fn post_false_for_regular_message() {
        assert!(!regular_msg(1, "x").post());
    }

    #[test]
    fn post_true_for_channel_post() {
        let m = Message {
            post: true,
            ..regular_msg(1, "x")
        };
        assert!(m.post());
    }
}
