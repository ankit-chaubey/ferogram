// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use ferogram::tl;
use ferogram::{Client, InputMessage, SignInError, TransportKind, update::Update};

const API_ID: i32 = 0;
const API_HASH: &str = "";
const PHONE: &str = "";
const BOT_TOKEN: &str = "";

// These values are sent to Telegram in InitConnection and appear
// in the active sessions list. Customize them for your app.
const DEVICE_MODEL: &str = "Desktop";
const SYSTEM_VERSION: &str = "Linux";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const LANG_CODE: &str = "en";
const SYSTEM_LANG_CODE: &str = "en";

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    if std::env::var("RUST_LOG").is_err() {
        // Safe: no other threads exist yet at this point in main.
        unsafe {
            std::env::set_var("RUST_LOG", "ferogram=warn");
        }
    }
    env_logger::init();
    if let Err(e) = run().await {
        eprintln!("\n✗ {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() {
        eprintln!("Set API_ID and API_HASH at the top of main.rs");
        std::process::exit(1);
    }
    println!("🔌 Connecting…");
    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .device_model(DEVICE_MODEL)
        .system_version(SYSTEM_VERSION)
        .app_version(APP_VERSION)
        .lang_code(LANG_CODE)
        .system_lang_code(SYSTEM_LANG_CODE)
        .transport(TransportKind::Abridged)
        .probe_transport(true)
        .resilient_connect(true)
        .connect()
        .await?;

    if !client.is_authorized().await? {
        do_login(&client).await?;
        client.save_session().await?;
        println!("💾 Session saved");
    } else {
        println!("✅ Already logged in");
    }

    let me = client.get_me().await?;
    println!(
        "👤 {} (id={}){}",
        full_name(&me),
        me.id,
        me.username
            .as_deref()
            .map(|u| format!(" @{u}"))
            .unwrap_or_default()
    );

    let t = Instant::now();
    match client
        .invoke(&tl::functions::Ping {
            ping_id: 0xDEAD_BEEF,
        })
        .await
    {
        Ok(tl::enums::Pong::Pong(p)) => println!(
            "🏓 Ping OK  rtt={}ms  msg_id={}",
            t.elapsed().as_millis(),
            p.msg_id
        ),
        Err(e) => println!("⚠️  Ping: {e}"),
    }
    println!("\n👂 Listening… (Ctrl+C to quit)\n");

    let client = Arc::new(client);
    let my_id = me.id;
    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        let c = client.clone();
        tokio::spawn(async move { dispatch(upd, &c, my_id).await });
    }
    Ok(())
}

async fn dispatch(upd: Update, client: &Client, my_id: i64) {
    match upd {
        Update::NewMessage(msg) => {
            let text = msg.text().unwrap_or("").trim().to_string();
            let out = msg.outgoing();
            if out && !text.starts_with('.') {
                return;
            }
            let Some(peer) = msg.peer_id() else { return };
            if is_self_peer(peer, my_id) && !text.starts_with('.') {
                return;
            }

            let sender_uid = uid_from_peer(msg.sender_id());
            let msg_id = msg.id();
            let peer = peer.clone();

            println!(
                "{}  [msg={}{}] {}",
                if out { "📤" } else { "📨" },
                msg_id,
                sender_uid
                    .map(|id| format!(" from={id}"))
                    .unwrap_or_default(),
                &text[..text.floor_char_boundary(100)]
            );

            if !text.starts_with('.') {
                return;
            }

            if let Some(uid) = sender_uid
                && uid != my_id
            {
                cache_sender(client, uid, msg_id, &peer).await;
            }

            let (cmd, arg) = split_cmd(&text);
            route(client, &cmd, &arg, peer, msg_id, my_id, sender_uid).await;
        }
        Update::MessageEdited(msg) => {
            println!("✏️  [msg={}] {}", msg.id(), msg.text().unwrap_or(""))
        }
        Update::MessageDeleted(del) => println!("🗑️  {:?}", del.message_ids),
        Update::Raw(raw) => println!("⚙️  {:#010x}", raw.constructor_id),
        _ => {}
    }
}

async fn route(
    client: &Client,
    cmd: &str,
    arg: &str,
    peer: tl::enums::Peer,
    msg_id: i32,
    _my_id: i64,
    sender: Option<i64>,
) {
    match cmd {
        ".ping" => cmd_ping(client, peer, msg_id).await,
        ".me" => cmd_me(client, peer, msg_id).await,
        ".id" => cmd_id(client, peer.clone(), msg_id, sender, &peer).await,
        ".msgid" => {
            rh(
                client,
                peer,
                msg_id,
                &format!("📌 <b>Msg ID:</b> <code>{msg_id}</code>"),
            )
            .await
        }
        ".dc" => cmd_dc(client, peer, msg_id).await,
        ".ferogram" => cmd_ferogram(client, peer, msg_id).await,
        ".time" => cmd_time(client, peer, msg_id).await,
        ".whois" => cmd_whois(client, peer.clone(), msg_id, sender, &peer).await,
        ".read" => {
            let _ = client.mark_as_read(peer).await;
        }
        ".del" => {
            let _ = client.delete_messages(&[msg_id], true).await;
        }
        ".pin" => {
            let _ = client.pin_message(peer, msg_id, true).await;
        }
        ".unpin" => {
            let _ = client.unpin_message(peer, msg_id).await;
        }
        ".typing" => {
            let _ = client
                .send_chat_action(peer, tl::enums::SendMessageAction::SendMessageTypingAction)
                .await;
        }
        ".dialogs" => cmd_dialogs(client, peer, msg_id).await,
        ".echo" => rp(client, peer, msg_id, arg).await,
        ".upper" => rp(client, peer, msg_id, &arg.to_uppercase()).await,
        ".lower" => rp(client, peer, msg_id, &arg.to_lowercase()).await,
        ".rev" => {
            let r: String = arg.chars().rev().collect();
            rp(client, peer, msg_id, &r).await;
        }
        ".count" => cmd_count(client, peer, msg_id, arg).await,
        ".calc" => cmd_calc(client, peer, msg_id, arg).await,
        ".edit" => {
            if !arg.is_empty() {
                let _ = client.edit_message(peer, msg_id, arg).await;
            }
        }
        ".fwd" => {
            if !arg.is_empty() {
                let _ = client.forward_messages(arg, &[msg_id], peer).await;
            }
        }
        ".help" => cmd_help(client, peer, msg_id).await,
        _ => {}
    }
}

async fn cache_sender(client: &Client, user_id: i64, msg_id: i32, chat_peer: &tl::enums::Peer) {
    let peer = match chat_peer {
        tl::enums::Peer::Chat(c) => {
            tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: c.chat_id })
        }
        _ => tl::enums::InputPeer::Empty,
    };
    let _ = client.get_user_from_message(peer, msg_id, user_id).await;
}

async fn cmd_ping(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let t = Instant::now();
    let ok = client
        .invoke(&tl::functions::Ping {
            ping_id: 0xDEAD_BEEF,
        })
        .await
        .is_ok();
    let rtt = t.elapsed().as_millis();
    rh(
        client,
        peer,
        reply_to,
        &if ok {
            format!("🏓 pong | <b>{rtt}ms</b>")
        } else {
            "🏓 pong | timeout".into()
        },
    )
    .await;
}

async fn cmd_me(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    match client.get_me().await {
        Ok(me) => rh(client, peer, reply_to, &format!(
            "👤 <b>Me</b>\n\n<b>Name:</b> {}\n<b>Username:</b> {}\n<b>ID:</b> <code>{}</code>\n<b>Phone:</b> <code>{}</code>\n<b>Premium:</b> {} <b>Bot:</b> {}",
            esc(&full_name(&me)),
            me.username.as_deref().map(|u| format!("@{u}")).unwrap_or_else(|| "none".into()),
            me.id, me.phone.as_deref().unwrap_or("hidden"),
            boo(me.premium), boo(me.bot),
        )).await,
        Err(e) => rp(client, peer, reply_to, &format!("❌ {e}")).await,
    }
}

async fn cmd_id(
    client: &Client,
    peer: tl::enums::Peer,
    reply_to: i32,
    sender: Option<i64>,
    chat_peer: &tl::enums::Peer,
) {
    let s = match sender {
        Some(id) => format!("<code>{id}</code>"),
        None => "unknown".into(),
    };
    let c = match chat_peer {
        tl::enums::Peer::User(u) => format!("<code>{}</code> (DM)", u.user_id),
        tl::enums::Peer::Chat(c) => format!("<code>{}</code> (group)", c.chat_id),
        tl::enums::Peer::Channel(c) => format!("<code>{}</code> (channel)", c.channel_id),
    };
    rh(client, peer, reply_to, &format!(
        "🪪 <b>IDs</b>\n\n<b>Sender:</b> {s}\n<b>Chat:</b> {c}\n<b>Msg:</b> <code>{reply_to}</code>"
    )).await;
}

async fn cmd_dc(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    match client.get_me().await {
        Ok(me) => rh(client, peer, reply_to, &format!(
            "🌐 <b>Connection</b>\n\n<b>Layer:</b> <code>{}</code>\n<b>User ID:</b> <code>{}</code>\n<b>Bot:</b> {}",
            tl::LAYER, me.id, boo(me.bot)
        )).await,
        Err(e) => rp(client, peer, reply_to, &format!("❌ {e}")).await,
    }
}

async fn cmd_ferogram(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    rh(client, peer, reply_to, &format!(
        "📡 <b>ferogram</b>\n\n<b>MTProto Layer:</b> <code>{}</code>\n<b>Crate:</b> <code>ferogram {}</code>\n<b>Language:</b> Rust 🦀\nhttps://github.com/ankit-chaubey/ferogram",
        tl::LAYER, env!("CARGO_PKG_VERSION")
    )).await;
}

async fn cmd_time(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let now = Utc::now();
    rh(client, peer, reply_to, &format!(
        "🕐 <b>Time</b>\n\n<b>Date:</b> {}\n<b>UTC:</b> <code>{}</code>\n<b>Unix:</b> <code>{}</code>",
        now.format("%A, %B %d %Y"), now.format("%H:%M:%S"), now.timestamp(),
    )).await;
}

async fn cmd_whois(
    client: &Client,
    peer: tl::enums::Peer,
    reply_to: i32,
    sender: Option<i64>,
    chat_peer: &tl::enums::Peer,
) {
    let Some(uid) = sender else {
        rp(client, peer, reply_to, "❓ Unknown sender.").await;
        return;
    };
    let peer_ctx = match chat_peer {
        tl::enums::Peer::Chat(c) => {
            tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: c.chat_id })
        }
        _ => tl::enums::InputPeer::Empty,
    };
    match client.get_user_from_message(peer_ctx, reply_to, uid).await {
        Ok(Some(u)) => {
            let f = u.first_name.as_deref().unwrap_or("");
            let l = u.last_name.as_deref().unwrap_or("");
            let uname = u
                .username
                .as_deref()
                .map(|s| format!("@{s}"))
                .unwrap_or_else(|| "none".into());
            rh(client, peer, reply_to, &format!(
                "👤 <b>User Info</b>\n\n<b>Name:</b> {}\n<b>Username:</b> {uname}\n<b>ID:</b> <code>{}</code>\n<b>Bot:</b> {} <b>Verified:</b> {} <b>Premium:</b> {}",
                esc(format!("{f} {l}").trim()), u.id, boo(u.bot), boo(u.verified), boo(u.premium),
            )).await;
        }
        Ok(None) => rp(client, peer, reply_to, "❓ User not found.").await,
        Err(e) => rp(client, peer, reply_to, &format!("❌ {e}")).await,
    }
}

async fn cmd_dialogs(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    match client.get_dialogs(10).await {
        Ok(dialogs) => {
            let mut lines = vec!["📋 <b>Recent Dialogs</b>\n".to_string()];
            for (i, d) in dialogs.iter().enumerate() {
                let u = d.unread_count();
                let badge = if u > 0 {
                    format!("  🔴{u}")
                } else {
                    String::new()
                };
                lines.push(format!("{}. {}{}", i + 1, esc(&d.title()), badge));
            }
            rh(client, peer, reply_to, &lines.join("\n")).await;
        }
        Err(e) => rp(client, peer, reply_to, &format!("❌ {e}")).await,
    }
}

async fn cmd_count(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: .count <text>").await;
        return;
    }
    rh(client, peer, reply_to, &format!(
        "📊 <b>Stats</b>\n\n<b>Chars:</b> <code>{}</code>\n<b>Bytes:</b> <code>{}</code>\n<b>Words:</b> <code>{}</code>\n<b>Lines:</b> <code>{}</code>",
        arg.chars().count(), arg.len(), arg.split_whitespace().count(), arg.lines().count(),
    )).await;
}

async fn cmd_calc(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: .calc <expr>").await;
        return;
    }
    let text = match eval(arg.trim()) {
        Ok(v) => format!("🧮 <code>{}</code> = <b>{v}</b>", esc(arg)),
        Err(e) => format!("❌ {e}"),
    };
    rh(client, peer, reply_to, &text).await;
}

async fn cmd_help(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    rh(client, peer, reply_to,
        "📖 <b>ferogram-app Commands</b>\n\n\
        <b>Info</b>\n\
        <code>.ping</code>: latency  <code>.me</code>: self info\n\
        <code>.id</code>: IDs  <code>.msgid</code>: msg ID\n\
        <code>.dc</code>: DC info  <code>.ferogram</code>: lib info\n\
        <code>.time</code>: UTC time  <code>.whois</code>: sender info\n\
        <code>.dialogs</code>: last 10 dialogs\n\n\
        <b>Actions</b>\n\
        <code>.read</code> <code>.del</code> <code>.pin</code> <code>.unpin</code> <code>.typing</code>\n\n\
        <b>Text</b>\n\
        <code>.echo</code> <code>.upper</code> <code>.lower</code> <code>.rev</code>\n\
        <code>.count &lt;text&gt;</code>  <code>.calc &lt;expr&gt;</code>\n\
        <code>.edit &lt;text&gt;</code>  <code>.fwd &lt;@peer&gt;</code>  <code>.help</code>"
    ).await;
}

async fn rp(client: &Client, peer: tl::enums::Peer, reply_to: i32, text: &str) {
    let _ = client
        .send_message(peer, InputMessage::text(text).reply_to(Some(reply_to)))
        .await;
}

async fn rh(client: &Client, peer: tl::enums::Peer, reply_to: i32, html: &str) {
    let _ = client
        .send_message(peer, InputMessage::html(html).reply_to(Some(reply_to)))
        .await;
}

fn is_self_peer(peer: &tl::enums::Peer, my_id: i64) -> bool {
    matches!(peer, tl::enums::Peer::User(u) if u.user_id == my_id)
}
fn uid_from_peer(peer: Option<&tl::enums::Peer>) -> Option<i64> {
    match peer? {
        tl::enums::Peer::User(u) => Some(u.user_id),
        _ => None,
    }
}
fn split_cmd(text: &str) -> (String, String) {
    match text.split_once(' ') {
        Some((c, r)) => (c.to_ascii_lowercase(), r.trim().to_string()),
        None => (text.to_ascii_lowercase(), String::new()),
    }
}
fn full_name(u: &tl::types::User) -> String {
    format!(
        "{} {}",
        u.first_name.as_deref().unwrap_or(""),
        u.last_name.as_deref().unwrap_or("")
    )
    .trim()
    .to_string()
}
fn boo(b: bool) -> &'static str {
    if b { "✅" } else { "❌" }
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn do_login(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    if !BOT_TOKEN.is_empty() {
        client.bot_sign_in(BOT_TOKEN).await?;
        return Ok(());
    }
    if PHONE.is_empty() {
        eprintln!("Set PHONE or BOT_TOKEN");
        std::process::exit(1);
    }
    let token = client.request_login_code(PHONE).await?;
    let code = prompt("Enter the code: ")?;
    match client.sign_in(&token, &code).await {
        Ok(name) => println!("✅ Signed in as {name}"),
        Err(SignInError::PasswordRequired(pw)) => {
            let pass = prompt(&format!(
                "2FA password (hint: {}): ",
                pw.hint().unwrap_or("?")
            ))?;
            client.check_password(*pw, pass.trim()).await?;
        }
        Err(SignInError::SignUpRequired) => {
            eprintln!("✗ Not registered.");
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn eval(expr: &str) -> Result<String, String> {
    for op in ['+', '-', '*', '/', '%'] {
        let from = if op == '-' { 1 } else { 0 };
        if let Some(pos) = expr[from..].rfind(op).map(|p| p + from) {
            let lhs: f64 = expr[..pos]
                .trim()
                .parse()
                .map_err(|_| format!("bad: '{}'", &expr[..pos].trim()))?;
            let rhs: f64 = expr[pos + 1..]
                .trim()
                .parse()
                .map_err(|_| format!("bad: '{}'", &expr[pos + 1..].trim()))?;
            let res = match op {
                '+' => lhs + rhs,
                '-' => lhs - rhs,
                '*' => lhs * rhs,
                '/' => {
                    if rhs == 0.0 {
                        return Err("div by zero".into());
                    }
                    lhs / rhs
                }
                '%' => {
                    if rhs == 0.0 {
                        return Err("mod by zero".into());
                    }
                    lhs % rhs
                }
                _ => unreachable!(),
            };
            return Ok(if res.fract() == 0.0 && res.abs() < 1e15 {
                format!("{}", res as i64)
            } else {
                format!("{res:.6}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            });
        }
    }
    expr.trim()
        .parse::<f64>()
        .map(|n| format!("{n}"))
        .map_err(|_| format!("cannot evaluate '{expr}'"))
}
