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

use std::time::Duration;

use crate::transport_kind::TransportKind;

/// One leg of a transport race: transport plus its start delay.
#[derive(Clone, Debug)]
pub struct RaceLeg {
    pub transport: TransportKind,
    pub stagger: Duration,
}

impl RaceLeg {
    pub fn new(transport: TransportKind, stagger_ms: u64) -> Self {
        Self {
            transport,
            stagger: Duration::from_millis(stagger_ms),
        }
    }
}

/// Full vs Obfuscated. Abridged/Intermediate aren't included since they
/// share Full's TCP path and framing fingerprint, so they live or die with
/// it against DPI - racing them adds load with no extra chance of success.
pub fn default_transport_race() -> Vec<RaceLeg> {
    vec![
        RaceLeg::new(TransportKind::Full, 0),
        RaceLeg::new(TransportKind::Obfuscated { secret: None }, 200),
    ]
}
