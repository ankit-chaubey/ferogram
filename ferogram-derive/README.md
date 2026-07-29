# ferogram-derive

Procedural macros for the ferogram workspace. Currently exposes `#[derive(FsmState)]`.

[![Crates.io](https://img.shields.io/crates/v/ferogram-derive?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-derive)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--derive-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-derive)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

Most people get this through `ferogram` with the `derive` feature flag. Direct usage is only needed when building on top of the FSM layer without the full client.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

---

## `#[derive(FsmState)]`

Implements the `ferogram::fsm::FsmState` trait for an enum. Only unit variants are supported; tuple or struct variants produce a compile error.

What gets generated: `as_key(&self) -> String` and `from_key(key: &str) -> Option<Self>`. Keys are namespaced as `"module::path::EnumName::Variant"` (using `module_path!()`, the enum name, and the variant name), so identically-named variants on different state enums -- or even identically-named enums in different modules -- don't collide in storage. Generic enums are rejected at compile time, since the key can't disambiguate different type parameter instantiations.

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

Renaming a variant, enum, or moving the enum to a different module changes its key and breaks any stored state. `from_key` falls back to matching the trailing `"::"`-segment against a variant name, so state written by pre-namespacing versions of this macro still deserializes on a best-effort basis after an upgrade.

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

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
