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

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use chrono::Utc;
use rand::{Rng, thread_rng};
use sha2::{Digest, Sha256};

use ferogram::tl;
use ferogram::{Client, InputMessage, TransportKind, update::Update};

const API_ID: i32 = 0;
const API_HASH: &str = "";
const BOT_TOKEN: &str = "";

// Proxy: paste a t.me/proxy link, or set to "" to disable.
// Supports all secret types: plain (hex), dd (padded), ee (fake-TLS).
// Example: "https://t.me/proxy?server=HOST&port=PORT&secret=SECRET"
const PROXY: &str = "";

// These values are sent to Telegram in InitConnection and appear
// in the active sessions list. Customize them for your bot.
const DEVICE_MODEL: &str = "Server";
const SYSTEM_VERSION: &str = "Linux";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const LANG_CODE: &str = "en";
const SYSTEM_LANG_CODE: &str = "en";

static MSG_COUNT: AtomicU64 = AtomicU64::new(0);
static START_TS: AtomicU64 = AtomicU64::new(0);

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "ferogram=warn");
        }
    }
    env_logger::init();
    if let Err(e) = run().await {
        eprintln!("✗ {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || BOT_TOKEN.is_empty() {
        eprintln!("Set API_ID, API_HASH and BOT_TOKEN at the top of src/main.rs");
        std::process::exit(1);
    }
    START_TS.store(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        Ordering::Relaxed,
    );

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
        .proxy_link(PROXY)
        .connect()
        .await?;

    if !client.is_authorized().await? {
        println!("🤖 Signing in…");
        client.bot_sign_in(BOT_TOKEN).await?;
        client.save_session().await?;
    }

    let me = client.get_me().await?;
    let bot_id = me.id;
    println!(
        "✅ @{} (id={bot_id}) ready: listening…\n",
        me.username.as_deref().unwrap_or("bot")
    );

    let client = Arc::new(client);
    let me = Arc::new(me);
    let mut stream = client.stream_updates();

    while let Some(upd) = stream.next().await {
        let c = client.clone();
        let me = me.clone();
        tokio::spawn(async move { dispatch(upd, c, me, bot_id).await });
    }
    Ok(())
}

async fn dispatch(upd: Update, client: Arc<Client>, me: Arc<tl::types::User>, bot_id: i64) {
    match upd {
        Update::NewMessage(msg) => {
            if msg.outgoing() {
                return;
            }
            if sender_uid(&msg) == Some(bot_id) {
                return;
            }
            let text = msg.text().unwrap_or("").trim().to_string();
            if !text.starts_with('/') {
                return;
            }
            let Some(peer) = msg.peer_id().cloned() else {
                return;
            };
            let msg_id = msg.id();
            let uid = sender_uid(&msg);
            MSG_COUNT.fetch_add(1, Ordering::Relaxed);
            println!(
                "📨 [msg={}{}] {}",
                msg_id,
                uid.map(|id| format!(" uid={id}")).unwrap_or_default(),
                &text[..text.len().min(80)]
            );

            let (cmd, arg) = split_cmd(&text, me.username.as_deref().unwrap_or(""));
            match cmd.as_deref() {
                Some("/start") => h_start(&client, peer, msg_id).await,
                Some("/help") => h_help(&client, peer, msg_id).await,
                Some("/ping") => h_ping(&client, peer, msg_id).await,
                Some("/id") => h_id(&client, peer.clone(), msg_id, uid, &peer).await,
                Some("/info") => h_info(&client, peer, msg_id, &me).await,
                Some("/time") => h_time(&client, peer, msg_id).await,
                Some("/stats") => h_stats(&client, peer, msg_id).await,
                Some("/ferogram") => h_ferogram(&client, peer, msg_id).await,
                Some("/about") => h_about(&client, peer, msg_id).await,
                Some("/echo") => h_echo(&client, peer, msg_id, &arg).await,
                Some("/upper") => h_tx(&client, peer, msg_id, &arg, |s| s.to_uppercase()).await,
                Some("/lower") => h_tx(&client, peer, msg_id, &arg, |s| s.to_lowercase()).await,
                Some("/reverse") => {
                    h_tx(&client, peer, msg_id, &arg, |s| s.chars().rev().collect()).await
                }
                Some("/rot13") => h_tx(&client, peer, msg_id, &arg, rot13).await,
                Some("/count") => h_count(&client, peer, msg_id, &arg).await,
                Some("/len") => {
                    h_simple(&client, peer, msg_id, &arg, "📏", |s| {
                        format!(
                            "<b>Length:</b> <code>{}</code> chars / <code>{}</code> bytes",
                            s.chars().count(),
                            s.len()
                        )
                    })
                    .await
                }
                Some("/b64enc") => {
                    h_simple(&client, peer, msg_id, &arg, "📦", |s| {
                        format!("<b>Base64:</b>\n<code>{}</code>", B64.encode(s.as_bytes()))
                    })
                    .await
                }
                Some("/b64dec") => h_b64dec(&client, peer, msg_id, &arg).await,
                Some("/hex") => {
                    h_simple(&client, peer, msg_id, &arg, "🔣", |s| {
                        format!(
                            "<b>Hex:</b>\n<code>{}</code>",
                            s.bytes()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    })
                    .await
                }
                Some("/unhex") => h_unhex(&client, peer, msg_id, &arg).await,
                Some("/hash") => {
                    h_simple(&client, peer, msg_id, &arg, "🔐", |s| {
                        format!("<b>SHA-256:</b>\n<code>{}</code>", sha256_hex(s.as_bytes()))
                    })
                    .await
                }
                Some("/morse") => h_morse(&client, peer, msg_id, &arg, true).await,
                Some("/unmorse") => h_morse(&client, peer, msg_id, &arg, false).await,
                Some("/calc") => h_calc(&client, peer, msg_id, &arg).await,
                Some("/roll") => h_roll(&client, peer, msg_id, &arg).await,
                Some("/flip") => {
                    rh(
                        &client,
                        peer,
                        msg_id,
                        if thread_rng().gen_bool(0.5) {
                            "🪙 <b>Heads!</b>"
                        } else {
                            "🟡 <b>Tails!</b>"
                        },
                    )
                    .await;
                }
                Some("/random") => h_random(&client, peer, msg_id, &arg).await,
                Some("/password") => h_password(&client, peer, msg_id, &arg).await,
                Some("/uuid") => {
                    rh(
                        &client,
                        peer,
                        msg_id,
                        &format!("🆔 <b>UUID v4:</b>\n<code>{}</code>", random_uuid()),
                    )
                    .await;
                }
                Some("/fact") => {
                    rh(
                        &client,
                        peer,
                        msg_id,
                        &format!("💡 <b>Fun Fact</b>\n\n{}", random_fact()),
                    )
                    .await;
                }
                Some("/joke") => {
                    rh(
                        &client,
                        peer,
                        msg_id,
                        &format!("😂 <b>Joke</b>\n\n{}", random_joke()),
                    )
                    .await;
                }
                Some("/fmt_md") => h_fmt_md(&client, peer, msg_id).await,
                Some("/fmt_html") => h_fmt_html(&client, peer, msg_id).await,
                _ => {
                    rp(
                        &client,
                        peer,
                        msg_id,
                        "❓ Unknown command. Use /help for all commands.",
                    )
                    .await;
                }
            }
        }

        Update::CallbackQuery(cb) => {
            let data = cb.data().unwrap_or("").to_string();
            let qid = cb.query_id;
            println!("🔘 callback [qid={qid}] data={data}");
            match data.as_str() {
                "cb:ping" => {
                    let _ = client
                        .answer_callback_query(qid, Some("🏓 Pong!"), false)
                        .await;
                }
                "cb:time" => {
                    let _ = client
                        .answer_callback_query(
                            qid,
                            Some(&Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()),
                            false,
                        )
                        .await;
                }
                "cb:stats" => {
                    let _ = client
                        .answer_callback_query(
                            qid,
                            Some(&format!(
                                "⏱ {} | 📨 {} msgs",
                                uptime(),
                                MSG_COUNT.load(Ordering::Relaxed)
                            )),
                            false,
                        )
                        .await;
                }
                "cb:ferogram" => {
                    let _ = client
                        .answer_callback_query(
                            qid,
                            Some(&format!("Layer {} · ferogram 0.1.1 🦀", tl::LAYER)),
                            false,
                        )
                        .await;
                }
                "cb:about" => {
                    let _ = client
                        .answer_callback_query(
                            qid,
                            Some("Built with ferogram: pure Rust MTProto 🦀"),
                            true,
                        )
                        .await;
                }
                "cb:help" => {
                    let _ = client
                        .answer_callback_query(qid, Some("Send /help for all commands"), false)
                        .await;
                }
                "cb:joke" => {
                    let _ = client
                        .answer_callback_query(qid, Some(random_joke()), true)
                        .await;
                }
                "cb:fact" => {
                    let _ = client
                        .answer_callback_query(qid, Some(random_fact()), true)
                        .await;
                }
                _ => {
                    let _ = client
                        .answer_callback_query(qid, Some("🤷 Unknown"), false)
                        .await;
                }
            }
        }

        Update::InlineQuery(iq) => {
            let q = iq.query().trim().to_string();
            let qid = iq.query_id;
            println!("🔍 inline [qid={qid}] q={q:?}");
            let results = if q.is_empty() {
                vec![
                    ia(
                        "1",
                        "🕐 Current time",
                        &Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    ),
                    ia(
                        "2",
                        "📡 Layer version",
                        &format!("Layer {} · ferogram 🦀", tl::LAYER),
                    ),
                    ia(
                        "3",
                        "🎲 Random 1–100",
                        &thread_rng().gen_range(1..=100u32).to_string(),
                    ),
                    ia(
                        "4",
                        "🪙 Coin flip",
                        if thread_rng().gen_bool(0.5) {
                            "Heads 🪙"
                        } else {
                            "Tails 🟡"
                        },
                    ),
                    ia("5", "😂 Joke", random_joke()),
                    ia("6", "💡 Fact", random_fact()),
                ]
            } else {
                let rev: String = q.chars().rev().collect();
                let rot = rot13(&q);
                let b64 = B64.encode(q.as_bytes());
                let hex: String = q.bytes().map(|b| format!("{b:02x}")).collect();
                let sha = sha256_hex(q.as_bytes());
                vec![
                    ia(
                        "u",
                        &format!("⬆️ UPPER: {}", q.to_uppercase()),
                        &q.to_uppercase(),
                    ),
                    ia(
                        "l",
                        &format!("⬇️ lower: {}", q.to_lowercase()),
                        &q.to_lowercase(),
                    ),
                    ia("r", &format!("🔃 Reversed: {rev}"), &rev),
                    ia("t", &format!("🔄 ROT-13: {rot}"), &rot),
                    ia("b", &format!("📦 Base64: {b64}"), &b64),
                    ia("h", &format!("🔣 Hex: {hex}"), &hex),
                    ia("s", &format!("🔐 SHA-256: {sha}"), &sha),
                    ia(
                        "c",
                        &format!(
                            "📊 {} chars, {} words",
                            q.chars().count(),
                            q.split_whitespace().count()
                        ),
                        &format!(
                            "{} characters, {} words",
                            q.chars().count(),
                            q.split_whitespace().count()
                        ),
                    ),
                ]
            };
            let _ = client
                .answer_inline_query(qid, results, 30, false, None)
                .await;
        }

        _ => {}
    }
}

async fn h_start(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let text = "👋 <b>Welcome to ferogram-bot!</b>\n\nShowcase bot built with <b>ferogram</b>: a pure-Rust async Telegram MTProto library 🦀\n\nUse the buttons below or send /help for all commands.";
    let kb = kb(vec![
        vec![bc("🏓 Ping", "cb:ping"), bc("🕐 Time", "cb:time")],
        vec![bc("📊 Stats", "cb:stats"), bc("📡 Layer", "cb:ferogram")],
        vec![bc("😂 Joke", "cb:joke"), bc("💡 Fact", "cb:fact")],
        vec![bc("📖 Help", "cb:help"), bc("ℹ️ About", "cb:about")],
        vec![bu("⭐ GitHub", "https://github.com/ankit-chaubey/ferogram")],
    ]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_help(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    rh(
        client,
        peer,
        reply_to,
        "📖 <b>ferogram-bot Commands</b>\n\n\
        <b>Core</b>\n/ping /id /info /time /stats /layer /about\n\n\
        <b>Text tools</b>\n\
        /echo /upper /lower /reverse /rot13 /count /len\n\n\
        <b>Encoding &amp; Crypto</b>\n\
        /b64enc /b64dec /hex /unhex /hash /morse /unmorse\n\n\
        <b>Math &amp; Random</b>\n\
        /calc /roll /flip /random /password /uuid\n\n\
        <b>Fun</b>\n/fact /joke\n\n\
        <b>Formatting demos</b>\n/fmt_md /fmt_html\n\n\
        <b>Inline:</b> <code>@bot &lt;text&gt;</code> in any chat",
    )
    .await;
}

async fn h_ping(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let t = Instant::now();
    let ok = client
        .invoke(&tl::functions::Ping {
            ping_id: 0xDEAD_BEEF,
        })
        .await
        .is_ok();
    let rtt = t.elapsed().as_millis();
    let text = if ok {
        format!("🏓 <b>Pong!</b> <code>{rtt}ms</code>")
    } else {
        "🏓 Pong! <i>(timeout)</i>".into()
    };
    let kb = kb(vec![vec![
        bc("🔁 Ping again", "cb:ping"),
        bc("🕐 Time", "cb:time"),
    ]]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(&text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_id(
    client: &Client,
    peer: tl::enums::Peer,
    reply_to: i32,
    uid: Option<i64>,
    chat_peer: &tl::enums::Peer,
) {
    let us = match uid {
        Some(id) => format!("<code>{id}</code>"),
        None => "unknown".into(),
    };
    let cs = match chat_peer {
        tl::enums::Peer::User(u) => format!("<code>{}</code> (DM)", u.user_id),
        tl::enums::Peer::Chat(c) => format!("<code>{}</code> (group)", c.chat_id),
        tl::enums::Peer::Channel(c) => format!("<code>{}</code> (channel)", c.channel_id),
    };
    rh(client, peer, reply_to, &format!(
        "🪪 <b>IDs</b>\n\n<b>Your ID:</b> {us}\n<b>Chat ID:</b> {cs}\n<b>Msg ID:</b> <code>{reply_to}</code>"
    )).await;
}

async fn h_info(client: &Client, peer: tl::enums::Peer, reply_to: i32, me: &tl::types::User) {
    rh(client, peer, reply_to, &format!(
        "🤖 <b>Bot Info</b>\n\n<b>Name:</b> {} {}\n<b>Username:</b> @{}\n<b>ID:</b> <code>{}</code>\n<b>Layer:</b> <code>{}</code>",
        esc(me.first_name.as_deref().unwrap_or("")),
        esc(me.last_name.as_deref().unwrap_or("")),
        me.username.as_deref().unwrap_or("none"),
        me.id, tl::LAYER,
    )).await;
}

async fn h_time(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let now = Utc::now();
    let text = format!(
        "🕐 <b>Time</b>\n\n<b>Date:</b> {}\n<b>UTC:</b> <code>{}</code>\n<b>Unix:</b> <code>{}</code>",
        now.format("%A, %B %d %Y"),
        now.format("%H:%M:%S"),
        now.timestamp(),
    );
    let kb = kb(vec![vec![bc("🔁 Refresh", "cb:time")]]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(&text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_stats(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let text = format!(
        "📊 <b>Bot Stats</b>\n\n<b>Uptime:</b> {}\n<b>Messages:</b> <code>{}</code>\n<b>Layer:</b> <code>{}</code>",
        uptime(),
        MSG_COUNT.load(Ordering::Relaxed),
        tl::LAYER,
    );
    let kb = kb(vec![vec![bc("🔁 Refresh", "cb:stats")]]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(&text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_ferogram(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let text = format!(
        "📡 <b>layer library</b>\n\n<b>MTProto Layer:</b> <code>{}</code>\n<b>Crate:</b> <code>ferogram 0.1.1</code>\n<b>Language:</b> Rust 🦀\nhttps://github.com/ankit-chaubey/ferogram",
        tl::LAYER
    );
    let kb = kb(vec![vec![
        bu("⭐ GitHub", "https://github.com/ankit-chaubey/ferogram"),
        bc("📡 Version", "cb:ferogram"),
    ]]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(&text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_about(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    let text = format!(
        "ℹ️ <b>About ferogram-bot</b>\n\nBuilt with <b>ferogram</b>: async Telegram MTProto in pure <b>Rust</b> 🦀\n\n\
        Commands · Inline keyboards · Callback queries · Inline mode · HTML entities · \
        Concurrent update handling · pts gap recovery\n\n\
        <b>Layer:</b> <code>{}</code>  <b>Author:</b> Ankit Chaubey (vasu)",
        tl::LAYER
    );
    let kb = kb(vec![
        vec![bu("⭐ GitHub", "https://github.com/ankit-chaubey/ferogram")],
        vec![bc("📊 Stats", "cb:stats"), bc("📡 Layer", "cb:ferogram")],
    ]);
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(&text)
                .reply_markup(kb)
                .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_echo(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "💬 Usage: /echo <text>").await;
        return;
    }
    rh(client, peer, reply_to, &format!("💬 {}", esc(arg))).await;
}

async fn h_tx<F: Fn(&str) -> String>(
    client: &Client,
    peer: tl::enums::Peer,
    reply_to: i32,
    arg: &str,
    f: F,
) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: <command> <text>").await;
        return;
    }
    rh(
        client,
        peer,
        reply_to,
        &format!("<code>{}</code>", esc(&f(arg))),
    )
    .await;
}

async fn h_simple<F: Fn(&str) -> String>(
    client: &Client,
    peer: tl::enums::Peer,
    reply_to: i32,
    arg: &str,
    emoji: &str,
    f: F,
) {
    if arg.is_empty() {
        rp(
            client,
            peer,
            reply_to,
            &format!("{emoji} Usage: <command> <text>"),
        )
        .await;
        return;
    }
    rh(client, peer, reply_to, &f(arg)).await;
}

async fn h_count(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: /count <text>").await;
        return;
    }
    rh(client, peer, reply_to, &format!(
        "📊 <b>Stats</b>\n\n<b>Chars:</b> <code>{}</code>\n<b>Bytes:</b> <code>{}</code>\n<b>Words:</b> <code>{}</code>\n<b>Lines:</b> <code>{}</code>",
        arg.chars().count(), arg.len(), arg.split_whitespace().count(), arg.lines().count(),
    )).await;
}

async fn h_b64dec(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: /b64dec <base64>").await;
        return;
    }
    let text = match B64.decode(arg.trim()) {
        Ok(b) => match String::from_utf8(b) {
            Ok(s) => format!("📦 <b>Decoded:</b>\n<code>{}</code>", esc(&s)),
            Err(_) => "❌ Not valid UTF-8.".into(),
        },
        Err(_) => "❌ Invalid Base64.".into(),
    };
    rh(client, peer, reply_to, &text).await;
}

async fn h_unhex(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(client, peer, reply_to, "Usage: /unhex <hex>").await;
        return;
    }
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        rp(client, peer, reply_to, "❌ Odd hex length.").await;
        return;
    }
    let text = match (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
    {
        Ok(b) => match String::from_utf8(b) {
            Ok(decoded) => format!("🔣 <b>Decoded:</b>\n<code>{}</code>", esc(&decoded)),
            Err(_) => "❌ Not valid UTF-8.".into(),
        },
        Err(_) => "❌ Invalid hex.".into(),
    };
    rh(client, peer, reply_to, &text).await;
}

async fn h_morse(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str, encode: bool) {
    if arg.is_empty() {
        rp(
            client,
            peer,
            reply_to,
            if encode {
                "Usage: /morse <text>"
            } else {
                "Usage: /unmorse <morse>"
            },
        )
        .await;
        return;
    }
    let text = if encode {
        match to_morse(arg) {
            Ok(m) => format!("🔔 <b>Morse:</b>\n<code>{m}</code>"),
            Err(e) => format!("❌ {e}"),
        }
    } else {
        match from_morse(arg) {
            Ok(t) => format!("🔔 <b>Decoded:</b>\n<code>{}</code>", esc(&t)),
            Err(e) => format!("❌ {e}"),
        }
    };
    rh(client, peer, reply_to, &text).await;
}

async fn h_calc(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    if arg.is_empty() {
        rp(
            client,
            peer,
            reply_to,
            "Usage: /calc <expr>  e.g. /calc 12 * 7",
        )
        .await;
        return;
    }
    let text = match eval(arg.trim()) {
        Ok(v) => format!("🧮 <code>{}</code> = <b>{v}</b>", esc(arg)),
        Err(e) => format!("❌ {e}"),
    };
    rh(client, peer, reply_to, &text).await;
}

async fn h_roll(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    let sides: u32 = arg.trim().parse().unwrap_or(6).clamp(2, 1_000_000);
    let roll = thread_rng().gen_range(1..=sides);
    rh(
        client,
        peer,
        reply_to,
        &format!("🎲 Rolling <b>d{sides}</b>… → <b>{roll}</b>"),
    )
    .await;
}

async fn h_random(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    let parts: Vec<i64> = arg
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let (lo, hi) = match parts.as_slice() {
        [a, b] => (*a.min(b), *a.max(b)),
        [a] => (1, *a),
        _ => (1, 100),
    };
    let n = thread_rng().gen_range(lo..=hi);
    rh(
        client,
        peer,
        reply_to,
        &format!("🎰 Random <code>{lo}..={hi}</code> → <b>{n}</b>"),
    )
    .await;
}

async fn h_password(client: &Client, peer: tl::enums::Peer, reply_to: i32, arg: &str) {
    let len: usize = arg.trim().parse().unwrap_or(16).clamp(4, 128);
    const CS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*-_=+?";
    let pwd: String = (0..len)
        .map(|_| CS[thread_rng().gen_range(0..CS.len())] as char)
        .collect();
    rh(
        client,
        peer,
        reply_to,
        &format!("🔑 <b>Password ({len} chars):</b>\n<code>{pwd}</code>"),
    )
    .await;
}

async fn h_fmt_md(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    // InputMessage::markdown() parses the markdown inline, no manual entity wiring needed.
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::markdown(
                "**Markdown example**\n\n\
                *bold* · _italic_ · ~~strike~~ · ||spoiler||\n\
                `inline code`\n\
                ```rust\nfn hello() { println!(\"hi\"); }\n```\n\
                [ferogram repo](https://github.com/ankit-chaubey/ferogram)",
            )
            .reply_to(Some(reply_to)),
        )
        .await;
}

async fn h_fmt_html(client: &Client, peer: tl::enums::Peer, reply_to: i32) {
    // InputMessage::html() parses the HTML inline, no manual entity wiring needed.
    let _ = client
        .send_message_to_peer_ex(
            peer,
            &InputMessage::html(
                "<b>HTML example</b>\n\n\
                <b>bold</b> · <i>italic</i> · <u>underline</u> · \
                <s>strike</s> · <tg-spoiler>spoiler</tg-spoiler>\n\
                <code>inline code</code>\n\
                <pre><code class=\"language-rust\">fn hello() { println!(\"hi\"); }</code></pre>\n\
                <a href=\"https://github.com/ankit-chaubey/ferogram\">ferogram repo</a>",
            )
            .reply_to(Some(reply_to)),
        )
        .await;
}

async fn rp(client: &Client, peer: tl::enums::Peer, reply_to: i32, text: &str) {
    let _ = client
        .send_message_to_peer_ex(peer, &InputMessage::text(text).reply_to(Some(reply_to)))
        .await;
}

async fn rh(client: &Client, peer: tl::enums::Peer, reply_to: i32, html: &str) {
    let _ = client
        .send_message_to_peer_ex(peer, &InputMessage::html(html).reply_to(Some(reply_to)))
        .await;
}

fn kb(rows: Vec<Vec<tl::enums::KeyboardButton>>) -> tl::enums::ReplyMarkup {
    tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup {
        rows: rows
            .into_iter()
            .map(|row| {
                tl::enums::KeyboardButtonRow::KeyboardButtonRow(tl::types::KeyboardButtonRow {
                    buttons: row,
                })
            })
            .collect(),
    })
}
fn bc(text: &str, data: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Callback(tl::types::KeyboardButtonCallback {
        requires_password: false,
        style: None,
        text: text.to_string(),
        data: data.as_bytes().to_vec(),
    })
}
fn bu(text: &str, url: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Url(tl::types::KeyboardButtonUrl {
        style: None,
        text: text.to_string(),
        url: url.to_string(),
    })
}
fn ia(id: &str, title: &str, content: &str) -> tl::enums::InputBotInlineResult {
    tl::enums::InputBotInlineResult::InputBotInlineResult(tl::types::InputBotInlineResult {
        id: id.to_string(),
        r#type: "article".to_string(),
        title: Some(title.to_string()),
        description: Some(content.to_string()),
        url: None,
        thumb: None,
        content: None,
        send_message: tl::enums::InputBotInlineMessage::Text(
            tl::types::InputBotInlineMessageText {
                no_webpage: false,
                invert_media: false,
                message: content.to_string(),
                entities: None,
                reply_markup: None,
            },
        ),
    })
}

fn split_cmd(text: &str, bot_username: &str) -> (Option<String>, String) {
    if !text.starts_with('/') {
        return (None, text.to_string());
    }
    let (cmd_raw, rest) = text
        .split_once(' ')
        .map(|(c, r)| (c, r.trim()))
        .unwrap_or((text, ""));
    let cmd = if let Some(pos) = cmd_raw.find('@') {
        if cmd_raw[pos + 1..].eq_ignore_ascii_case(bot_username) {
            &cmd_raw[..pos]
        } else {
            cmd_raw
        }
    } else {
        cmd_raw
    };
    (Some(cmd.to_ascii_lowercase()), rest.to_string())
}

fn sender_uid(msg: &ferogram::update::IncomingMessage) -> Option<i64> {
    match msg.sender_id()? {
        tl::enums::Peer::User(u) => Some(u.user_id),
        _ => None,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='m' | 'A'..='M' => (c as u8 + 13) as char,
            'n'..='z' | 'N'..='Z' => (c as u8 - 13) as char,
            o => o,
        })
        .collect()
}
fn uptime() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(START_TS.load(Ordering::Relaxed));
    format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
}
fn random_uuid() -> String {
    let mut b = [0u8; 16];
    thread_rng().fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let node: u64 = (b[10] as u64) << 40
        | (b[11] as u64) << 32
        | (b[12] as u64) << 24
        | (b[13] as u64) << 16
        | (b[14] as u64) << 8
        | (b[15] as u64);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]),
        u16::from_be_bytes([b[8], b[9]]),
        node
    )
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

const MORSE: &[(&str, &str)] = &[
    ("a", ".-"),
    ("b", "-..."),
    ("c", "-.-."),
    ("d", "-.."),
    ("e", "."),
    ("f", "..-."),
    ("g", "--."),
    ("h", "...."),
    ("i", ".."),
    ("j", ".---"),
    ("k", "-.-"),
    ("l", ".-.."),
    ("m", "--"),
    ("n", "-."),
    ("o", "---"),
    ("p", ".--."),
    ("q", "--.-"),
    ("r", ".-."),
    ("s", "..."),
    ("t", "-"),
    ("u", "..-"),
    ("v", "...-"),
    ("w", ".--"),
    ("x", "-..-"),
    ("y", "-.--"),
    ("z", "--.."),
    ("0", "-----"),
    ("1", ".----"),
    ("2", "..---"),
    ("3", "...--"),
    ("4", "....-"),
    ("5", "....."),
    ("6", "-...."),
    ("7", "--..."),
    ("8", "---.."),
    ("9", "----."),
];
fn to_morse(text: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for ch in text.to_lowercase().chars() {
        if ch == ' ' {
            out.push("/".to_string());
            continue;
        }
        match MORSE.iter().find(|(k, _)| *k == ch.to_string().as_str()) {
            Some((_, m)) => out.push(m.to_string()),
            None => return Err(format!("No Morse for '{ch}'")),
        }
    }
    Ok(out.join(" "))
}
fn from_morse(morse: &str) -> Result<String, String> {
    let mut out = String::new();
    for word in morse.trim().split(" / ") {
        for code in word.split_whitespace() {
            match MORSE.iter().find(|(_, m)| *m == code) {
                Some((ch, _)) => out.push_str(ch),
                None => return Err(format!("Unknown code '{code}'")),
            }
        }
        out.push(' ');
    }
    Ok(out.trim().to_string())
}

fn random_fact() -> &'static str {
    const F: &[&str] = &[
        "Honey never spoils. Archaeologists found 3000-year-old honey still perfectly edible.",
        "A day on Venus is longer than a year on Venus.",
        "Octopuses have three hearts and blue blood.",
        "The Eiffel Tower grows up to 15 cm in summer due to thermal expansion.",
        "Bananas are berries, but strawberries are not.",
        "Sharks are older than trees by about 50 million years.",
        "The shortest war in history lasted 38 minutes (Anglo-Zanzibar War, 1896).",
        "Wombat poop is cube-shaped.",
        "A bolt of lightning is 5× hotter than the surface of the Sun.",
        "Crows can recognise and remember human faces.",
        "Cleopatra lived closer to the Moon landing than to the pyramids' construction.",
        "There are more possible chess games than atoms in the observable universe.",
    ];
    F[thread_rng().gen_range(0..F.len())]
}
fn random_joke() -> &'static str {
    const J: &[&str] = &[
        "Why do programmers prefer dark mode? Because light attracts bugs.",
        "A SQL query walks into a bar and asks two tables: 'Can I join you?'",
        "There are 10 types of people: those who understand binary and those who don't.",
        "Why do Java developers wear glasses? Because they don't C#.",
        "I have a joke about UDP, but I'm not sure you'll get it.",
        "I have a joke about TCP, but I'll keep sending it until you acknowledge it.",
        "Debugging is being the detective in a crime movie where you're also the murderer.",
        "It's not a bug: it's an undocumented feature.",
        "Why did the Rust developer cross the road? To avoid garbage collection.",
        "A byte walks into a bar looking rough. Bartender: 'What's wrong?' Byte: 'Bit error.'",
    ];
    J[thread_rng().gen_range(0..J.len())]
}
