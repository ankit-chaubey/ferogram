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
        tracing::debug!("[ferogram::connect] MTProxy: opening TCP connection to {addr}");
        TcpStream::connect(&addr).await.map_err(ConnectError::Io)
    }

    /// Socket address string `"host:port"`.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
