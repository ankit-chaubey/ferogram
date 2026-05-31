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

//! Tests for Release 6: typed transfer errors and structured tracing.
//!
//! Run:
//!   cargo run --example typed_errors
//!
//! No Telegram connection required. All assertions run locally against the
//! type system and error conversion logic.
//!
//! What is covered:
//!
//!   1. TransferError::Cancelled is returned when handle.cancel() fires
//!   2. TransferError::Cancelled converts to InvocationError cleanly
//!   3. InvocationError converts back to TransferError::Cancelled
//!   4. ErrorKind::Cancelled comes from .kind() on a cancel InvocationError
//!   5. TransferError::FloodWait round-trips through InvocationError correctly
//!   6. TransferError::Rpc round-trips correctly
//!   7. TransferError::Network wraps io::Error correctly
//!   8. TransferHandle cancel + poll_pause_cancel integration
//!   9. Tracing subscriber captures ferogram::transfer target events

use ferogram::{ErrorKind, InvocationErrorExt, TransferError, TransferHandle};

#[allow(unused_assignments)]
fn main() {
    // Set up tracing so span/event output is visible during the run.
    tracing_subscriber::fmt()
        .with_target(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferogram::transfer=trace".parse().unwrap()),
        )
        .init();

    let mut passed = 0usize;
    let mut failed = 0usize;

    macro_rules! check {
        ($label:expr, $cond:expr) => {
            if $cond {
                println!("  ok  {}", $label);
                passed += 1;
            } else {
                println!("FAIL  {}", $label);
                failed += 1;
            }
        };
    }

    println!("\n--- TransferError display ---");
    check!(
        "Cancelled display",
        TransferError::Cancelled.to_string() == "transfer cancelled by caller"
    );
    check!(
        "FloodWait display",
        TransferError::FloodWait { seconds: 42 }.to_string()
            == "Telegram rate limit reached. Retry after 42 seconds."
    );
    check!(
        "Rpc display",
        TransferError::Rpc {
            code: 400,
            name: "FILE_PART_INVALID".into()
        }
        .to_string()
            == "Telegram error (400): FILE_PART_INVALID"
    );
    {
        let io = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        let s = TransferError::Network(io).to_string();
        check!(
            "Network display contains 'network error'",
            s.contains("network error")
        );
    }

    println!("\n--- TransferError -> InvocationError (From) ---");
    {
        use ferogram::InvocationError;
        let inv: InvocationError = TransferError::Cancelled.into();
        check!(
            "Cancelled -> InvocationError::Deserialize(cancel)",
            matches!(&inv, InvocationError::Deserialize(s) if s.contains("cancel"))
        );
    }
    {
        use ferogram::InvocationError;
        let inv: InvocationError = TransferError::FloodWait { seconds: 120 }.into();
        check!(
            "FloodWait -> InvocationError::Rpc code 420",
            matches!(&inv, InvocationError::Rpc(r) if r.code == 420)
        );
        check!(
            "FloodWait -> value is 120",
            matches!(&inv, InvocationError::Rpc(r) if r.value == Some(120))
        );
    }
    {
        use ferogram::InvocationError;
        let inv: InvocationError = TransferError::Rpc {
            code: 400,
            name: "FILE_PART_INVALID".into(),
        }
        .into();
        check!(
            "Rpc -> InvocationError::Rpc code 400",
            matches!(&inv, InvocationError::Rpc(r) if r.code == 400)
        );
    }
    {
        use ferogram::InvocationError;
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        let inv: InvocationError = TransferError::Network(io).into();
        check!(
            "Network -> InvocationError::Io",
            matches!(&inv, InvocationError::Io(_))
        );
    }

    println!("\n--- InvocationError -> TransferError (From) ---");
    {
        use ferogram::InvocationError;
        let inv = InvocationError::Deserialize("transfer cancelled by caller".into());
        let te: TransferError = inv.into();
        check!(
            "cancel Deserialize -> TransferError::Cancelled",
            matches!(te, TransferError::Cancelled)
        );
    }
    {
        use ferogram::{InvocationError, RpcError};
        let inv = InvocationError::Rpc(RpcError {
            code: 420,
            name: "FLOOD_WAIT_60".into(),
            value: Some(60),
        });
        let te: TransferError = inv.into();
        check!(
            "FLOOD_WAIT Rpc -> TransferError::FloodWait",
            matches!(te, TransferError::FloodWait { seconds: 60 })
        );
    }
    {
        use ferogram::InvocationError;
        let io = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe");
        let inv = InvocationError::Io(io);
        let te: TransferError = inv.into();
        check!(
            "Io -> TransferError::Network",
            matches!(te, TransferError::Network(_))
        );
    }

    println!("\n--- ErrorKind::Cancelled via .kind() ---");
    {
        use ferogram::InvocationError;
        let inv: InvocationError = TransferError::Cancelled.into();
        check!(
            ".kind() on cancel InvocationError == ErrorKind::Cancelled",
            inv.kind() == ErrorKind::Cancelled
        );
    }
    {
        use ferogram::{InvocationError, RpcError};
        let inv = InvocationError::Rpc(RpcError {
            code: 420,
            name: "FLOOD_WAIT_30".into(),
            value: Some(30),
        });
        check!(
            ".kind() on FLOOD_WAIT == ErrorKind::FloodWait(30)",
            inv.kind() == ErrorKind::FloodWait(30)
        );
    }

    println!("\n--- TransferHandle cancel + poll_pause_cancel ---");
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = TransferHandle::new();
        handle.cancel();
        let result = rt.block_on(handle.poll_pause_cancel());
        check!(
            "poll_pause_cancel returns Err(Cancelled) after cancel()",
            matches!(result, Err(TransferError::Cancelled))
        );
    }
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = TransferHandle::new();
        // Not cancelled, not paused: should return Ok immediately.
        let result = rt.block_on(handle.poll_pause_cancel());
        check!(
            "poll_pause_cancel returns Ok(()) when not cancelled",
            result.is_ok()
        );
    }
    {
        // Cancel fires while paused: should still return Cancelled.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = TransferHandle::new();
        handle.pause();
        let h2 = handle.clone();
        rt.block_on(async move {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                h2.cancel();
            });
            let result = handle.poll_pause_cancel().await;
            check!(
                "poll_pause_cancel returns Cancelled when cancel fires during pause",
                matches!(result, Err(TransferError::Cancelled))
            );
        });
    }

    println!("\n--- ? operator propagation through From impl ---");
    {
        // Simulate what upload_resumable does: function returns InvocationError,
        // poll_pause_cancel returns TransferError, ? converts via From.
        fn simulate_upload(cancelled: bool) -> Result<(), ferogram::InvocationError> {
            let handle = TransferHandle::new();
            if cancelled {
                handle.cancel();
            }
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(handle.poll_pause_cancel())?; // TransferError -> InvocationError via ?
            Ok(())
        }
        check!(
            "? on poll_pause_cancel in InvocationError context propagates cancel",
            simulate_upload(true).is_err()
        );
        check!(
            "? on poll_pause_cancel in InvocationError context passes when not cancelled",
            simulate_upload(false).is_ok()
        );
    }

    println!();
    println!("results: {} passed, {} failed", passed, failed);
    if failed > 0 {
        std::process::exit(1);
    }
}
