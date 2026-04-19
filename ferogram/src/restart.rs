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

use std::time::Duration;

pub trait ConnectionRestartPolicy: Send + Sync + 'static {
    fn restart_interval(&self) -> Option<Duration>;
}

pub struct NeverRestart;

impl ConnectionRestartPolicy for NeverRestart {
    fn restart_interval(&self) -> Option<Duration> {
        None
    }
}

pub struct FixedInterval {
    pub interval: Duration,
}

impl ConnectionRestartPolicy for FixedInterval {
    fn restart_interval(&self) -> Option<Duration> {
        Some(self.interval)
    }
}
