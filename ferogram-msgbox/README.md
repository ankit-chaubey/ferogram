# ferogram-msgbox

Update-gap tracking state machine for ferogram: pts/qts/seq bookkeeping,
per-channel state, and gap detection for Telegram's `Updates` stream.

[![Crates.io](https://img.shields.io/crates/v/ferogram-msgbox?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-msgbox)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--msgbox-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-msgbox)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

`ferogram` re-exports this as `ferogram::message_box`, so most people never
depend on it directly. If you're just building a bot or a client, start with
[`ferogram`](https://crates.io/crates/ferogram) instead.

## What it does

Telegram's `Updates` stream is lossy over the wire (pushed updates can drop
or arrive out of order) but the protocol carries a `pts`/`qts`/`seq` counter
in every update, so a client can detect exactly when it missed something and
ask for a diff to fill the hole. `MessageBoxes` is the state machine that
does that bookkeeping:

- Tracks the global `pts`/`qts`/`seq`/`date` counters, plus a separate `pts`
  per channel
- Detects gaps (an update arrives with a `pts_count` that doesn't chain onto
  the last known `pts`) and buffers out-of-order updates for a short window
  before giving up and requesting a diff
- Decides when a periodic catch-up `getDifference` is due, even with no
  known gap (`NO_UPDATES_TIMEOUT`, 15 minutes)
- Classifies your own outgoing RPC responses (`classify_own_response`) so
  updates piggybacked on them - e.g. `sendMessage` returning the new
  message - get folded into the same pts sequence
- Applies `updates.getDifference` / `updates.getChannelDifference` results
  back into state once the caller has fetched them

It's a **pure state machine**: no async, no networking, no RPC calls. It
takes an `UpdatesLike` in via `process_updates()` and hands back either the
update batch (plus any referenced users/chats) or a `Gap`, which tells the
caller which RPC to run and how to feed the result back.

## Usage

```rust
use ferogram_msgbox::{MessageBoxes, UpdatesLike};

let mut mbox = MessageBoxes::new();

// Feed a pushed update frame (or an own-RPC response) in.
match mbox.process_updates(UpdatesLike::Updates(Box::new(incoming))) {
    Ok((updates, users, chats)) => {
        // dispatch each Vec's items as usual.
    }
    Err(_gap) => {
        // A gap was detected (or a diff was already due). Fetch it and
        // feed the result back:
        if let Some(req) = mbox.get_difference() {
            let diff = client.invoke(&req).await?; // tl::enums::updates::Difference
            let (updates, users, chats) = mbox.apply_difference(diff);
            // dispatch updates/users/chats
        }
    }
}

// Called periodically (e.g. every tick of your update loop) even with no
// known gap - handles the 15-minute no-updates safety net and any pending
// per-channel diffs.
let deadline = mbox.check_deadlines();
```

On reconnect, feed `UpdatesLike::ConnectionClosed` in - anything sent while
the socket was down (or before you were listening) is exactly the kind of
gap this is designed to catch.

## Stack position

```
ferogram
└ ferogram-msgbox  <-- here (used directly, not layered under mtsender/connect)
  └ ferogram-tl-types
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
