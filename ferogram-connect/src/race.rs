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
