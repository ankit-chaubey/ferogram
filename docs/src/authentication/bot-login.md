# Bot Login

Bot login is simpler than user login: just a single call with a bot token.

## Getting a bot token

1. Open Telegram and start a chat with [@BotFather](https://t.me/BotFather)
2. Send `/newbot`
3. Follow the prompts to choose a name and username
4. BotFather gives you a token like: `1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ`

## Login

```rust
client.bot_sign_in("1234567890:ABCdef...").await?;
client.save_session().await?;
```

That's it. On the next run, `is_authorized()` returns `true` and you skip the login entirely:

```rust
if !client.is_authorized().await? {
    client.bot_sign_in(BOT_TOKEN).await?;
    client.save_session().await?;
}
```

## Get bot info

After login you can fetch the bot's own User object:

```rust
let me = client.get_me().await?;
println!("Bot: @{}", me.username.as_deref().unwrap_or("?"));
println!("ID: {}", me.id);
println!("Is bot: {}", me.bot);
```

## Environment variables (recommended)

Don't hardcode credentials in source code. Use environment variables instead:

```rust
let api_id: i32   = std::env::var("API_ID")?.parse()?;
let api_hash      = std::env::var("API_HASH")?;
let bot_token     = std::env::var("BOT_TOKEN")?;
```

Then run:

```bash
API_ID=12345 API_HASH=abc123 BOT_TOKEN=xxx:yyy cargo run
```

Or put them in a `.env` file and use the `dotenvy` crate.
