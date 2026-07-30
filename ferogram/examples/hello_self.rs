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

//! Sends "Hello from ferogram!" to your Saved Messages (the "me" chat).
//!
//! Run:
//!   cargo run --example hello_self
//!
//! First run will prompt for your phone number or bot token,
//! then the login code, then 2FA password if you have one set.
//! After that the session is saved so subsequent runs skip all of that.

use ferogram::Client;

const API_ID: i32 = 0; // fill in your api_id from https://my.telegram.org
const API_HASH: &str = ""; // fill in your api_hash from https://my.telegram.org

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() {
        eprintln!("Fill in API_ID and API_HASH at the top of hello_self.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("hello.session", API_ID, API_HASH).await?;

    client.send_message("me", "Hello from ferogram!").await?;
    println!("Message sent to Saved Messages.");

    client.save_session().await?;
    Ok(())
}
