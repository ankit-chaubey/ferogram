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

use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;

use crate::error::ConnectError;

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
    pub async fn connect(&self, target: &str) -> Result<TcpStream, ConnectError> {
        tracing::debug!(
            "[ferogram::connect] SOCKS5: relaying through {} to {target}",
            self.proxy_addr
        );
        let stream = match &self.auth {
            None => Socks5Stream::connect(self.proxy_addr.as_str(), target)
                .await
                .map_err(|e| ConnectError::Io(std::io::Error::other(e)))?,
            Some((user, pass)) => Socks5Stream::connect_with_password(
                self.proxy_addr.as_str(),
                target,
                user.as_str(),
                pass.as_str(),
            )
            .await
            .map_err(|e| ConnectError::Io(std::io::Error::other(e)))?,
        };
        Ok(stream.into_inner())
    }
}
