// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use ferogram_tl_types as tl;
use std::sync::Mutex;

use crate::errors::InvocationError;

/// Return the statically known IPv4 address for a Telegram DC.
///
/// Used as a fallback when the DC is not yet in the session's dc_options table
/// (i.e. first migration to a DC we haven't talked to before).
///
/// Source: https://core.telegram.org/mtproto/DC
pub fn fallback_dc_addr(dc_id: i32) -> &'static str {
    match dc_id {
        1 => "149.154.175.53:443",
        2 => "149.154.167.51:443",
        3 => "149.154.175.100:443",
        4 => "149.154.167.91:443",
        5 => "91.108.56.130:443",
        _ => "149.154.167.51:443",
    }
}

/// Build the initial DC options map from the static table.
pub fn default_dc_addresses() -> Vec<(i32, String)> {
    (1..=5)
        .map(|id| (id, fallback_dc_addr(id).to_string()))
        .collect()
}

// Tracks which DCs already have a copy of the auth to avoid redundant round-trips.

/// State that must live inside `ClientInner` to track which DCs already have
/// a copy of the account's authorization key.
pub struct DcAuthTracker {
    copied: Mutex<Vec<i32>>,
}

impl DcAuthTracker {
    pub fn new() -> Self {
        Self {
            copied: Mutex::new(Vec::new()),
        }
    }

    /// Check if we have already copied auth to `dc_id`.
    pub fn has_copied(&self, dc_id: i32) -> bool {
        self.copied.lock().unwrap().contains(&dc_id)
    }

    /// Mark `dc_id` as having received a copy of the auth.
    pub fn mark_copied(&self, dc_id: i32) {
        self.copied.lock().unwrap().push(dc_id);
    }
}

impl Default for DcAuthTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Export the home-DC authorization and import it on `target_dc_id`.
///
/// A no-op if:
/// - `target_dc_id == home_dc_id` (already home)
/// - auth was already copied in this session (tracked by `DcAuthTracker`)
///
/// Ported from  `Client::copy_auth_to_dc`.
///
/// # Where to call this
///
/// Call from `invoke_on_dc(target_dc_id, req)` before sending the request,
/// so that file downloads on foreign DCs work without manual setup:
///
/// ```rust,ignore
/// pub async fn invoke_on_dc<R: RemoteCall>(
/// &self,
/// dc_id: i32,
/// req: &R,
/// ) -> Result<R::Return, InvocationError> {
/// self.copy_auth_to_dc(dc_id).await?;
/// // ... then call the DC-specific connection
/// }
/// ```
pub async fn copy_auth_to_dc<F, Fut>(
    home_dc_id: i32,
    target_dc_id: i32,
    tracker: &DcAuthTracker,
    invoke_fn: F, // calls the home DC
    invoke_on_dc_fn: impl Fn(i32, tl::functions::auth::ImportAuthorization) -> Fut,
) -> Result<(), InvocationError>
where
    F: std::future::Future<
            Output = Result<tl::enums::auth::ExportedAuthorization, InvocationError>,
        >,
    Fut: std::future::Future<Output = Result<tl::enums::auth::Authorization, InvocationError>>,
{
    if target_dc_id == home_dc_id {
        return Ok(());
    }
    if tracker.has_copied(target_dc_id) {
        return Ok(());
    }

    let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) = invoke_fn.await?;

    invoke_on_dc_fn(
        target_dc_id,
        tl::functions::auth::ImportAuthorization {
            id: exported.id,
            bytes: exported.bytes,
        },
    )
    .await?;

    tracker.mark_copied(target_dc_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dcs_return_correct_ips() {
        assert_eq!(fallback_dc_addr(1), "149.154.175.53:443");
        assert_eq!(fallback_dc_addr(2), "149.154.167.51:443");
        assert_eq!(fallback_dc_addr(3), "149.154.175.100:443");
        assert_eq!(fallback_dc_addr(4), "149.154.167.91:443");
        assert_eq!(fallback_dc_addr(5), "91.108.56.130:443");
    }

    #[test]
    fn unknown_dc_falls_back_to_dc2() {
        assert_eq!(fallback_dc_addr(99), "149.154.167.51:443");
    }

    #[test]
    fn default_dc_addresses_has_five_entries() {
        let addrs = default_dc_addresses();
        assert_eq!(addrs.len(), 5);
        // DCs 1-5 are all present
        for id in 1..=5_i32 {
            assert!(addrs.iter().any(|(dc_id, _)| *dc_id == id));
        }
    }

    #[test]
    fn tracker_starts_empty() {
        let t = DcAuthTracker::new();
        assert!(!t.has_copied(2));
        assert!(!t.has_copied(4));
    }

    #[test]
    fn tracker_marks_and_checks() {
        let t = DcAuthTracker::new();
        t.mark_copied(4);
        assert!(t.has_copied(4));
        assert!(!t.has_copied(2));
    }

    #[test]
    fn tracker_marks_multiple_dcs() {
        let t = DcAuthTracker::new();
        t.mark_copied(2);
        t.mark_copied(4);
        t.mark_copied(5);
        assert!(t.has_copied(2));
        assert!(t.has_copied(4));
        assert!(t.has_copied(5));
        assert!(!t.has_copied(1));
        assert!(!t.has_copied(3));
    }

    #[test]
    fn rpc_error_migrate_detection_all_variants() {
        use crate::errors::RpcError;

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
    fn invocation_error_migrate_dc_id_delegates() {
        use crate::errors::{InvocationError, RpcError};
        let e = InvocationError::Rpc(RpcError {
            code: 303,
            name: "PHONE_MIGRATE".into(),
            value: Some(5),
        });
        assert_eq!(e.migrate_dc_id(), Some(5));
    }
}
