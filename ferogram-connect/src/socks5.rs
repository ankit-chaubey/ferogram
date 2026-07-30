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

use tokio::net::TcpStream;
#[cfg(feature = "socks5")]
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
    #[cfg(feature = "socks5")]
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

    /// Establish a TCP connection through this SOCKS5 proxy.
    ///
    /// Returns an error: the "socks5" feature is disabled, so no SOCKS5
    /// client is compiled in.
    #[cfg(not(feature = "socks5"))]
    pub async fn connect(&self, _target: &str) -> Result<TcpStream, ConnectError> {
        Err(ConnectError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SOCKS5 proxy requested but ferogram-connect was built without the \"socks5\" feature",
        )))
    }
}
