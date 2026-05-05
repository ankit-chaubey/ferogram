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

/// Connect to Telegram test DC2, do full DH, invoke GetNearestDc, assert response.
/// Requires network access. Run with: cargo test -p ferogram-mtsender -- --ignored
#[tokio::test]
#[ignore]
async fn test_invoke_on_test_dc() {
    // connect to 149.154.167.40:443
    // DH handshake via ferogram_mtproto::authentication
    // invoke InvokeWithLayer { InitConnection { GetNearestDc } }
    // assert Ok(NearestDc)
    todo!("implement when network access available")
}
