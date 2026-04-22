// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::sync::Arc;
use std::time::Duration;

use ferogram::{Client, ClientBuilder, FsmState, UpdateStream};
use ferogram::filters::{Dispatcher, Router, command, text, private, group};
use ferogram::fsm::{MemoryStorage, StateContext};
use ferogram::middleware::{TracingMiddleware, RateLimitMiddleware, PanicRecoveryMiddleware};

// FSM state enum

#[derive(FsmState, Clone, Debug, PartialEq)]
enum OrderState {
    WaitingProduct,
    WaitingQuantity,
    WaitingAddress,
    Confirm,
}

// Routers

pub fn order_router() -> Router {
    let mut r = Router::new().scope(private());

    r.on_message(command("order"), handle_order_start);

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
    r.on_message(command("help"),  handle_help);
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
    msg.reply("👋 Welcome! Use /order to place an order.").await.ok();
}

async fn handle_help(msg: ferogram::update::IncomingMessage) {
    msg.reply("/order - start a new order\n/cancel - cancel current order").await.ok();
}

async fn handle_rules(msg: ferogram::update::IncomingMessage) {
    msg.reply("📋 Group rules: be respectful.").await.ok();
}

async fn handle_order_start(msg: ferogram::update::IncomingMessage) {
    msg.reply("🛍 What product would you like to order?").await.ok();
    // The first on_message_fsm handler fires once we set state.
    // State is set via a StateContext obtained from a prior message.
    // Typically you'd set the initial state here via a storage handle.
    // For demo purposes the user can set state externally.
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

    let product:  Option<String> = state.get_data("product").await.unwrap_or(None);
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
            let product:  Option<String> = state.get_data("product").await.unwrap_or(None);
            let quantity: Option<String> = state.get_data("quantity").await.unwrap_or(None);
            let address:  Option<String> = state.get_data("address").await.unwrap_or(None);

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
            msg.reply("❓ Reply 'yes' to confirm or /cancel to abort.").await.ok();
        }
    }
}

// Main

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let client = ClientBuilder::from_env()?.connect().await?;

    let storage = Arc::new(MemoryStorage::new());

    let mut dp = Dispatcher::new();

    // Middleware - registration order = execution order (outer → inner).
    dp.middleware(PanicRecoveryMiddleware::new()); // outermost: catches panics in all inner layers
    dp.middleware(TracingMiddleware::new());
    dp.middleware(RateLimitMiddleware::new(10, Duration::from_secs(1)));

    // FSM backend.
    dp.with_state_storage(Arc::clone(&storage));

    // Include routers.
    dp.include(info_router());
    dp.include(order_router());
    dp.include(group_router());

    tracing::info!("bot started");

    let mut stream: UpdateStream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        dp.dispatch(upd).await;
    }

    Ok(())
}
