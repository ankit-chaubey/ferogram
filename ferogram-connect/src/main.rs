// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Telegram MTProto full auth key generation + encrypted API demo.
//!
//! # What this does
//!
//! 1. TCP connect to Telegram DC1 (test servers by default)
//! 2. **Step 1**: send `req_pq_multi`
//! 3. **Step 2**: receive `ResPQ`, factorize PQ, RSA-encrypt → `req_DH_params`
//! 4. **Step 3**: receive `ServerDhParams`, complete DH → `set_client_DH_params`
//! 5. **Finish**: receive `DhGenOk`, derive `AuthKey`
//! 6. Call `help.getConfig` using MTProto 2.0 encrypted transport
//! 7. Print the DC list from the config
//!
//! # Run
//! ```
//! cargo run -p ferogram-connect
//! ```

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[allow(unused_imports)]
use ferogram_mtproto::transport::{AbridgedTransport, ObfuscatedAbridged, Transport};
use ferogram_mtproto::{EncryptedSession, Session, authentication as auth};
use ferogram_tl_types::{Cursor, Deserializable};

/// Production DC1
#[allow(dead_code)]
const DC1_PROD: &str = "149.154.167.51:443";

/// Test DC1 (use while developing)
#[allow(dead_code)]
const DC1_TEST: &str = "149.154.167.40:80";

struct Tcp(TcpStream);

impl Tcp {
    fn connect(addr: &str) -> std::io::Result<Self> {
        let s = TcpStream::connect(addr)?;
        s.set_read_timeout(Some(Duration::from_secs(15)))?;
        s.set_write_timeout(Some(Duration::from_secs(15)))?;
        Ok(Self(s))
    }
}

impl Transport for Tcp {
    type Error = std::io::Error;
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(data)
    }
    fn recv(&mut self) -> Result<Vec<u8>, Self::Error> {
        let mut first = [0u8; 1];
        self.0.read_exact(&mut first)?;
        let words = if first[0] < 0x7f {
            first[0] as usize
        } else {
            let mut buf = [0u8; 3];
            self.0.read_exact(&mut buf)?;
            buf[0] as usize | (buf[1] as usize) << 8 | (buf[2] as usize) << 16
        };
        let mut payload = vec![0u8; words * 4];
        self.0.read_exact(&mut payload)?;
        Ok(payload)
    }
}

fn plaintext_body(frame: &[u8]) -> Result<&[u8], &'static str> {
    if frame.len() < 20 {
        return Err("frame too short");
    }
    if u64::from_le_bytes(frame[..8].try_into().unwrap()) != 0 {
        return Err("auth_key_id != 0 in plaintext response");
    }
    let len = u32::from_le_bytes(frame[16..20].try_into().unwrap()) as usize;
    if frame.len() < 20 + len {
        return Err("truncated body");
    }
    Ok(&frame[20..20 + len])
}

fn send_plain<T: ferogram_tl_types::RemoteCall>(
    transport: &mut ObfuscatedAbridged,
    session: &mut Session,
    call: &T,
) -> std::io::Result<()> {
    let msg = session.pack(call);
    transport.send_message(&msg.to_plaintext_bytes())
}

fn recv_plain<T: Deserializable>(
    transport: &mut ObfuscatedAbridged,
) -> Result<T, Box<dyn std::error::Error>> {
    let raw = transport.recv_message()?;
    let body = plaintext_body(&raw)?;
    let mut cur = Cursor::from_slice(body);
    Ok(T::deserialize(&mut cur)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Connect
    println!("Connecting to {} …", DC1_PROD);
    let tcp = Tcp::connect(DC1_PROD)?;
    let mut transport = ObfuscatedAbridged::new(tcp.0)?;
    let mut session = Session::new();
    println!("✓ TCP connected");

    // 2. Auth key: Step 1: req_pq_multi
    let (req1, state1) = auth::step1()?;
    println!("\n[Step 1] Sending req_pq_multi …");
    send_plain(&mut transport, &mut session, &req1)?;

    let res_pq: ferogram_tl_types::enums::ResPq = recv_plain(&mut transport)?;
    let ferogram_tl_types::enums::ResPq::ResPq(pq) = &res_pq;
    println!("  ✓ ResPQ: pq={:02x?}", pq.pq);

    // 3. Auth key: Step 2: req_DH_params
    let (req2, state2) = auth::step2(state1, res_pq, 1)?;
    println!("[Step 2] Sending req_DH_params …");
    send_plain(&mut transport, &mut session, &req2)?;

    let server_dh: ferogram_tl_types::enums::ServerDhParams = recv_plain(&mut transport)?;
    match &server_dh {
        ferogram_tl_types::enums::ServerDhParams::Ok(_) => println!("  ✓ ServerDhParamsOk"),
        ferogram_tl_types::enums::ServerDhParams::Fail(_) => println!("  ✗ ServerDhParamsFail"),
    }

    // 4. Auth key: Step 3: set_client_DH_params
    let (req3, state3) = auth::step3(state2, server_dh)?;
    println!("[Step 3] Sending set_client_DH_params …");
    send_plain(&mut transport, &mut session, &req3)?;

    let dh_answer: ferogram_tl_types::enums::SetClientDhParamsAnswer = recv_plain(&mut transport)?;

    // 5. Derive auth key
    let done = match auth::finish(state3, dh_answer)? {
        auth::FinishResult::Done(f) => f,
        auth::FinishResult::Retry { .. } => return Err("DH handshake needs retry".into()),
    };
    println!("\n✓ Auth key derived!");
    println!("  time_offset = {}s", done.time_offset);
    println!("  first_salt  = {}", done.first_salt);
    println!("  auth_key    = {:02x?}…", &done.auth_key[..8]);

    // 6. Encrypted session: call help.getConfig
    println!("\n[Encrypted] Calling help.getConfig …");
    let mut enc = EncryptedSession::new(done.auth_key, done.first_salt, done.time_offset);

    let get_config = ferogram_tl_types::functions::help::GetConfig {};
    let wire = enc.pack(&get_config);
    transport.send_message(&wire)?;

    let mut raw = transport.recv_message()?;

    match enc.unpack(&mut raw) {
        Ok(msg) => {
            let mut cur = Cursor::from_slice(&msg.body);
            match ferogram_tl_types::enums::Config::deserialize(&mut cur) {
                Ok(ferogram_tl_types::enums::Config::Config(cfg)) => {
                    println!(
                        "  ✓ Config: {} DCs, date={}",
                        cfg.dc_options.len(),
                        cfg.date
                    );
                    println!("  DC list (first 5):");
                    for dc in cfg.dc_options.iter().take(5) {
                        let ferogram_tl_types::enums::DcOption::DcOption(opt) = dc;
                        println!("    DC{} {}:{}", opt.id, opt.ip_address, opt.port);
                    }
                }
                Err(e) => println!("  ⚠ Config parse failed: {e}"),
            }
        }
        Err(e) => println!("  ⚠ Decryption failed: {e}"),
    }

    println!("\n✓ Full MTProto flow complete!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_body_ok() {
        let mut f = vec![0u8; 24];
        f[16..20].copy_from_slice(&4u32.to_le_bytes());
        f[20..24].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(plaintext_body(&f).unwrap(), &[1, 2, 3, 4]);
    }

    #[test]
    fn plaintext_body_rejects_short() {
        assert!(plaintext_body(&[0u8; 10]).is_err());
    }

    #[test]
    fn plaintext_body_rejects_encrypted() {
        let mut f = [0u8; 24];
        f[0] = 1;
        assert!(plaintext_body(&f).is_err());
    }
}
