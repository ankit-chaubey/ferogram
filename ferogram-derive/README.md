# ferogram-derive

Procedural macros for the ferogram workspace. Currently exposes `#[derive(FsmState)]`.

[![Crates.io](https://img.shields.io/crates/v/ferogram-derive?color=fc8d62)](https://crates.io/crates/ferogram-derive)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--derive-5865F2)](https://docs.rs/ferogram-derive)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Most people get this through `ferogram` with the `derive` feature flag. Direct usage is only needed when building on top of the FSM layer without the full client.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

---

## `#[derive(FsmState)]`

Implements the `ferogram::fsm::FsmState` trait for an enum. Only unit variants are supported; tuple or struct variants produce a compile error.

What gets generated: `as_key(&self) -> String` and `from_key(key: &str) -> Option<Self>`, both using the variant name as written in source.

```rust
use ferogram::FsmState;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum RegistrationState {
    Start,
    WaitingName,
    WaitingPhone,
    Done,
}
```

Keys are the variant names as written. Renaming a variant changes its key and breaks any stored state.

---

## Using FsmState in a dispatcher

```rust
use ferogram::{FsmState, fsm::MemoryStorage, filters::text};
use std::sync::Arc;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum Form { Name, Age, Done }

dp.with_state_storage(Arc::new(MemoryStorage::new()));

dp.on_message_fsm(text(), Form::Name, |msg, state| async move {
    state.set_data("name", msg.text().unwrap()).await.ok();
    state.transition(Form::Age).await.ok();
    msg.reply("How old are you?").await.ok();
});

dp.on_message_fsm(text(), Form::Age, |msg, state| async move {
    let name = state.get_data("name").await.unwrap_or_default();
    state.transition(Form::Done).await.ok();
    msg.reply(format!("Got it, {name}!")).await.ok();
});
```

---

## Stack position

```
ferogram
└ ferogram-derive  <-- here (proc-macro crate, compile-time only)
```

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
