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

//! SOCKS5 proxy connector.
//!
//! Provides [`Socks5Config`] that can be attached to a [`crate::Config`]
//! so every Telegram connection is routed through a SOCKS5 proxy.
//!
//! # Example
//! ```rust,no_run
//! use ferogram::{Config, proxy::Socks5Config};
//! use std::sync::Arc;
//! use ferogram::retry::AutoSleep;
//!
//! let cfg = Config {
//! socks5: Some(Socks5Config::new("127.0.0.1:1080")),
//! ..Default::default()
//! };
//! ```

use crate::InvocationError;
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;

/// SOCKS5 proxy configuration.
#[derive(Clone, Debug)]
pub struct Socks5Config {
    /// Host:port of the SOCKS5 proxy server.
    pub proxy_addr: String,
    /// Optional username and password for proxy authentication.
    pub auth: Option<(String, String)>,
}

impl Socks5Config {
    /// Create an unauthenticated SOCKS5 config.
    pub fn new(proxy_addr: impl Into<String>) -> Self {
        Self {
            proxy_addr: proxy_addr.into(),
            auth: None,
        }
    }

    /// Create a SOCKS5 config with username/password authentication.
    pub fn with_auth(
        proxy_addr: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            proxy_addr: proxy_addr.into(),
            auth: Some((username.into(), password.into())),
        }
    }

    /// Establish a TCP connection through this SOCKS5 proxy.
    ///
    /// Returns a [`TcpStream`] tunnelled through the proxy to `target`.
    pub async fn connect(&self, target: &str) -> Result<TcpStream, InvocationError> {
        tracing::info!("[socks5] Connecting via {} → {target}", self.proxy_addr);
        let stream = match &self.auth {
            None => Socks5Stream::connect(self.proxy_addr.as_str(), target)
                .await
                .map_err(|e| InvocationError::Io(std::io::Error::other(e)))?,
            Some((user, pass)) => Socks5Stream::connect_with_password(
                self.proxy_addr.as_str(),
                target,
                user.as_str(),
                pass.as_str(),
            )
            .await
            .map_err(|e| InvocationError::Io(std::io::Error::other(e)))?,
        };
        tracing::info!("[socks5] Connected ✓");
        Ok(stream.into_inner())
    }
}
