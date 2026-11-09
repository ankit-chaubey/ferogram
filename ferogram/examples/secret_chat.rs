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

use ferogram::Client;
use ferogram::e2e::{E2eClient, E2eClientExt, E2eEvent};

const API_ID: i32 = 0; // fill in your api_id from https://my.telegram.org
const API_HASH: &str = ""; // fill in your api_hash from https://my.telegram.org

// Who to open a secret chat with: a @username, phone number or user id.
const TARGET: &str = "";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || TARGET.is_empty() {
        eprintln!("Fill in API_ID, API_HASH, and TARGET at the top of secret_chat.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("e2e.session", API_ID, API_HASH).await?;

    // Keeps chats in `e2e.e2e` next to the session, so they survive restarts.
    let e2e = E2eClient::for_session(client.clone(), "e2e.session")?;

    let chat_id = e2e.request_secret_chat(TARGET).await?;
    println!("requested secret chat, waiting id={chat_id}");

    let mut stream = client.stream_updates();
    while let Some(update) = stream.next().await {
        match e2e.handle_client_update(&update).await {
            Ok(Some(E2eEvent::ChatReady { chat_id })) => {
                println!("chat ready: {chat_id}");
                e2e.send_text(chat_id, "hello from ferogram-e2e", 0).await?;
                println!("sent!");
                break;
            }
            Ok(Some(E2eEvent::Message {
                chat_id, message, ..
            })) => {
                println!("got message in {chat_id}: {message:?}");
            }
            Ok(Some(e)) => println!("event: {e:?}"),
            Ok(None) => {}
            Err(e) => eprintln!("e2e error: {e}"),
        }
    }

    client.save_session().await?;
    Ok(())
}
