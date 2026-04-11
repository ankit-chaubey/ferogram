# Quick Start: Bot

A production-ready bot skeleton with commands, callback queries, and inline mode: all handled concurrently.

```rust
use ferogram::{Client, InputMessage, parsers::parse_markdown, update::Update};
use ferogram_tl_types as tl;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, _shutdown) = Client::builder()
        .api_id(std::env::var("API_ID")?.parse()?)
        .api_hash(std::env::var("API_HASH")?)
        .session("bot.session")
        .connect()
        .await?;
    let client = Arc::new(client);

    if !client.is_authorized().await? {
        client.bot_sign_in(&std::env::var("BOT_TOKEN")?).await?;
        client.save_session().await?;
    }

    let me = client.get_me().await?;
    println!("✅ @{} is online", me.username.as_deref().unwrap_or("bot"));

    let mut updates = client.stream_updates();

    while let Some(update) = updates.next().await {
        let client = client.clone();
        // Each update in its own task: the loop never blocks
        tokio::spawn(async move {
            if let Err(e) = dispatch(update, &client).await {
                eprintln!("Handler error: {e}");
            }
        });
    }

    Ok(())
}

async fn dispatch(
    update: Update,
    client: &Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

    match update {
        // Commands
        Update::NewMessage(msg) if !msg.outgoing() => {
            let text = msg.text().unwrap_or("").trim().to_string();
            let peer = match msg.peer_id() {
                Some(p) => p.clone(),
                None    => return Ok(()),
            };
            let reply_to = msg.id();

            if !text.starts_with('/') { return Ok(()); }

            let cmd = text.split_whitespace().next().unwrap_or("");
            let arg = text[cmd.len()..].trim();

            match cmd {
                "/start" => {
                    let (t, e) = parse_markdown(
                        "👋 **Hello!** I'm built with **ferogram**: async Telegram MTProto in Rust 🦀\n\n\
                         Use /help to see all commands."
                    );
                    client.send_message_to_peer_ex(peer, &InputMessage::text(t)
                        .entities(e).reply_to(Some(reply_to))).await?;
                }
                "/help" => {
                    let (t, e) = parse_markdown(
                        "📖 **Commands**\n\n\
                         /start: Welcome message\n\
                         /ping: Latency check\n\
                         /echo `<text>`: Repeat your text\n\
                         /upper `<text>`: UPPERCASE\n\
                         /lower `<text>`: lowercase\n\
                         /reverse `<text>`: esreveR\n\
                         /id: Your user and chat ID"
                    );
                    client.send_message_to_peer_ex(peer, &InputMessage::text(t)
                        .entities(e).reply_to(Some(reply_to))).await?;
                }
                "/ping" => {
                    let start = std::time::Instant::now();
                    client.send_message_to_peer(peer.clone(), "🏓 …").await?;
                    let ms = start.elapsed().as_millis();
                    let (t, e) = parse_markdown(&format!("🏓 **Pong!** `{ms} ms`"));
                    client.send_message_to_peer_ex(peer, &InputMessage::text(t)
                        .entities(e).reply_to(Some(reply_to))).await?;
                }
                "/echo"    => { client.send_message_to_peer(peer, if arg.is_empty() { "Usage: /echo <text>" } else { arg }).await?; }
                "/upper"   => { client.send_message_to_peer(peer, &arg.to_uppercase()).await?; }
                "/lower"   => { client.send_message_to_peer(peer, &arg.to_lowercase()).await?; }
                "/reverse" => {
                    let rev: String = arg.chars().rev().collect();
                    client.send_message_to_peer(peer, &rev).await?;
                }
                "/id" => {
                    let chat = match &peer {
                        tl::enums::Peer::User(u)    => format!("User `{}`",    u.user_id),
                        tl::enums::Peer::Chat(c)    => format!("Group `{}`",   c.chat_id),
                        tl::enums::Peer::Channel(c) => format!("Channel `{}`", c.channel_id),
                    };
                    let (t, e) = parse_markdown(&format!("🪪 **Chat:** {chat}"));
                    client.send_message_to_peer_ex(peer, &InputMessage::text(t)
                        .entities(e).reply_to(Some(reply_to))).await?;
                }
                _ => { client.send_message_to_peer(peer, "❓ Unknown command. Try /help").await?; }
            }
        }

        // Callback queries
        Update::CallbackQuery(cb) => {
            match cb.data().unwrap_or("") {
                "help"  => { client.answer_callback_query(cb.query_id, Some("Send /help for commands"), false).await?; }
                "about" => { client.answer_callback_query(cb.query_id, Some("Built with ferogram: Rust MTProto 🦀"), true).await?; }
                _       => { client.answer_callback_query(cb.query_id, None, false).await?; }
            }
        }

        // Inline mode
        Update::InlineQuery(iq) => {
            let q   = iq.query().to_string();
            let qid = iq.query_id;
            let results = vec![
                make_article("1", "🔠 UPPER", &q.to_uppercase()),
                make_article("2", "🔡 lower", &q.to_lowercase()),
                make_article("3", "🔄 Reversed", &q.chars().rev().collect::<String>()),
            ];
            client.answer_inline_query(qid, results, 30, false, None).await?;
        }

        _ => {}
    }

    Ok(())
}

fn make_article(id: &str, title: &str, text: &str) -> tl::enums::InputBotInlineResult {
    tl::enums::InputBotInlineResult::InputBotInlineResult(tl::types::InputBotInlineResult {
        id: id.into(), r#type: "article".into(),
        title: Some(title.into()), description: Some(text.into()),
        url: None, thumb: None, content: None,
        send_message: tl::enums::InputBotInlineMessage::Text(
            tl::types::InputBotInlineMessageText {
                no_webpage: false, invert_media: false,
                message: text.into(), entities: None, reply_markup: None,
            }
        ),
    })
}
```

---

## Key differences: User vs Bot

| Capability | User account | Bot |
|---|---|---|
| Login method | Phone + code + optional 2FA | Bot token from @BotFather |
| Read all messages | ✅ In any joined chat | ❌ Only messages directed at it |
| Send to any peer | ✅ | ❌ User must start the bot first |
| Inline mode | ❌ | ✅ `@botname query` in any chat |
| Callback queries | ✅ | ✅ |
| Anonymous in groups | ❌ | ✅ If admin |
| Rate limits | Stricter | More generous |

---

## Next steps

- [Bot Login: full guide](./authentication/bot-login.md)
- [Callback Queries](./updates/callbacks.md)
- [Inline Mode](./updates/inline-mode.md)
- [Inline Keyboards](./messaging/keyboards.md)
- [Admin & Ban Rights](./api/admin-rights.md)
