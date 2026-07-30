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
