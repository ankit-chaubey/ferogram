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

use crate::error::ConnectError;
use crate::transport_kind::TransportKind;

/// Decoded MTProxy configuration.
#[derive(Clone, Debug)]
pub struct MtProxyConfig {
    /// Proxy server hostname or IP.
    pub host: String,
    /// Proxy server port.
    pub port: u16,
    /// Raw secret bytes.
    pub secret: Vec<u8>,
    /// Transport variant; pass this as `config.transport`.
    pub transport: TransportKind,
}

impl MtProxyConfig {
    /// Open a TCP connection to the MTProxy host:port.
    pub async fn connect(&self) -> Result<TcpStream, ConnectError> {
        let addr = format!("{}:{}", self.host, self.port);
        tracing::debug!("[ferogram] MTProxy TCP connect -> {addr}");
        TcpStream::connect(&addr).await.map_err(ConnectError::Io)
    }

    /// Socket address string `"host:port"`.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
