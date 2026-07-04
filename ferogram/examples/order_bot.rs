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

use std::time::Duration;

use ferogram::filters::{Dispatcher, Router, command, group, private, text};
use ferogram::fsm::{MemoryStorage, StateContext, StateKey, StateKeyStrategy, StateStorage};
use ferogram::middleware::{PanicRecoveryMiddleware, RateLimitMiddleware, TracingMiddleware};
use ferogram::{ClientBuilder, FsmState, UpdateStream};

// FSM state enum

#[derive(FsmState, Clone, Debug, PartialEq)]
enum OrderState {
    WaitingProduct,
    WaitingQuantity,
    WaitingAddress,
    Confirm,
}

// Routers

pub fn order_router(storage: std::sync::Arc<dyn StateStorage>) -> Router {
    let mut r = Router::new().scope(private());

    r.on_message(command("order"), move |msg| {
        let storage = std::sync::Arc::clone(&storage);
        async move { handle_order_start(msg, storage).await }
    });

    r.on_message_fsm(text(), OrderState::WaitingProduct, handle_product);
    r.on_message_fsm(text(), OrderState::WaitingQuantity, handle_quantity);
    r.on_message_fsm(text(), OrderState::WaitingAddress, handle_address);
    r.on_message_fsm(text(), OrderState::Confirm, handle_confirm);

    // /cancel works from any FSM state.
    for state in [
        OrderState::WaitingProduct,
        OrderState::WaitingQuantity,
        OrderState::WaitingAddress,
        OrderState::Confirm,
    ] {
        r.on_message_fsm(command("cancel"), state, |msg, ctx| async move {
            ctx.clear_all().await.ok();
            msg.reply("❌ Order cancelled.").await.ok();
        });
    }

    r
}

pub fn info_router() -> Router {
    let mut r = Router::new();
    r.on_message(command("start"), handle_start);
    r.on_message(command("help"), handle_help);
    r
}

pub fn group_router() -> Router {
    // All handlers in this router only fire in groups.
    let mut r = Router::new().scope(group());
    r.on_message(command("rules"), handle_rules);
    r
}

// Handlers

async fn handle_start(msg: ferogram::update::IncomingMessage) {
    msg.reply("👋 Welcome! Use /order to place an order.")
        .await
        .ok();
}

async fn handle_help(msg: ferogram::update::IncomingMessage) {
    msg.reply("/order - start a new order\n/cancel - cancel current order")
        .await
        .ok();
}

async fn handle_rules(msg: ferogram::update::IncomingMessage) {
    msg.reply("📋 Group rules: be respectful.").await.ok();
}

/// Entry point for `/order`. This is a plain `on_message` handler (not
/// `on_message_fsm`), so it runs with no pre-existing state -- its job is
/// to *create* the first state for this conversation slot. Every
/// `on_message_fsm(..., OrderState::X, ...)` handler after this one only
/// fires once its expected state matches what we set here.
async fn handle_order_start(
    msg: ferogram::update::IncomingMessage,
    storage: std::sync::Arc<dyn StateStorage>,
) {
    // Must use the same StateKeyStrategy the Dispatcher uses (the default,
    // per-user-per-chat, unless you called `dp.with_key_strategy(...)`) --
    // otherwise this key won't match the one on_message_fsm looks up.
    let key = StateKey::from_message(&msg, StateKeyStrategy::default());

    if let Err(e) = storage
        .set_state(key, OrderState::WaitingProduct.as_key())
        .await
    {
        tracing::error!("failed to start order flow: {e}");
        msg.reply("⚠️ Couldn't start your order, try again.")
            .await
            .ok();
        return;
    }

    msg.reply("🛍 What product would you like to order?")
        .await
        .ok();
}

async fn handle_product(msg: ferogram::update::IncomingMessage, state: StateContext) {
    let product = msg.text().unwrap_or("unknown");
    state.set_data("product", product).await.ok();
    state.transition(OrderState::WaitingQuantity).await.ok();
    msg.reply("📦 How many would you like?").await.ok();
}

async fn handle_quantity(msg: ferogram::update::IncomingMessage, state: StateContext) {
    let qty = msg.text().unwrap_or("1");
    state.set_data("quantity", qty).await.ok();
    state.transition(OrderState::WaitingAddress).await.ok();
    msg.reply("🏠 What's your shipping address?").await.ok();
}

async fn handle_address(msg: ferogram::update::IncomingMessage, state: StateContext) {
    let addr = msg.text().unwrap_or("unknown");
    state.set_data("address", addr).await.ok();
    state.transition(OrderState::Confirm).await.ok();

    let product: Option<String> = state.get_data("product").await.unwrap_or(None);
    let quantity: Option<String> = state.get_data("quantity").await.unwrap_or(None);

    msg.reply(format!(
        "🧾 Confirm order?\n\nProduct: {}\nQty: {}\nTo: {}\n\nReply 'yes' to confirm or /cancel.",
        product.unwrap_or_default(),
        quantity.unwrap_or_default(),
        addr,
    ))
    .await
    .ok();
}

async fn handle_confirm(msg: ferogram::update::IncomingMessage, state: StateContext) {
    match msg.text().unwrap_or("").to_lowercase().trim() {
        "yes" | "confirm" | "ok" => {
            let product: Option<String> = state.get_data("product").await.unwrap_or(None);
            let quantity: Option<String> = state.get_data("quantity").await.unwrap_or(None);
            let address: Option<String> = state.get_data("address").await.unwrap_or(None);

            state.clear_all().await.ok();

            msg.reply(format!(
                "✅ Order placed!\n\n{} × {} → {}",
                quantity.unwrap_or_default(),
                product.unwrap_or_default(),
                address.unwrap_or_default(),
            ))
            .await
            .ok();
        }
        _ => {
            msg.reply("❓ Reply 'yes' to confirm or /cancel to abort.")
                .await
                .ok();
        }
    }
}

// Main

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_id: i32 = std::env::var("API_ID")?.parse()?;
    let api_hash = std::env::var("API_HASH")?;
    let bot_token = std::env::var("BOT_TOKEN")?;

    let (client, _shutdown) = ClientBuilder::default()
        .api_id(api_id)
        .api_hash(api_hash)
        .session("order_bot.session")
        .connect()
        .await?;

    if !client.is_authorized().await? {
        client.bot_sign_in(&bot_token).await?;
    }

    let storage: std::sync::Arc<dyn StateStorage> = std::sync::Arc::new(MemoryStorage::new());

    let mut dp = Dispatcher::new();

    // Middleware - registration order = execution order (outer → inner).
    dp.middleware(PanicRecoveryMiddleware::new()); // outermost: catches panics in all inner layers
    dp.middleware(TracingMiddleware::new());
    dp.middleware(RateLimitMiddleware::new(10, Duration::from_secs(1)));

    // Include routers. order_router needs its own handle to the same
    // storage so /order can create the first state; with_state_storage
    // below gives the Dispatcher the handle it uses to read states back.
    dp.include(info_router());
    dp.include(order_router(std::sync::Arc::clone(&storage)));
    dp.include(group_router());

    // FSM backend.
    dp.with_state_storage(storage);

    tracing::info!("bot started");

    let mut stream: UpdateStream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        dp.dispatch(upd).await;
    }

    Ok(())
}
