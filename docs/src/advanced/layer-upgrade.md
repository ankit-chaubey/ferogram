# Upgrading the TL Layer

The Telegram API evolves continuously. Each new **ferogram** adds constructors, modifies existing types, and deprecates old ones. Upgrading `ferogram` is designed to be a one-file operation.

## How the system works

`ferogram-tl-types` is fully **auto-generated at build time**:

```
tl/api.tl          (source of truth: the only file you replace)
    │
    ▼
build.rs           (reads api.tl, invokes ferogram-tl-gen)
    │
    ▼
$OUT_DIR/
  generated_common.rs     ← pub const LAYER: i32 = 224;
  generated_types.rs      ← pub mod types { ... }
  generated_enums.rs      ← pub mod enums { ... }
  generated_functions.rs  ← pub mod functions { ... }
```

The `LAYER` constant is extracted from the `// LAYER N` comment on the first line of `api.tl`. Everything else flows from there.

## Step 1: Replace api.tl

```bash
# Get the new schema from Telegram's official sources
# (TDLib repository, core.telegram.org, or unofficial mirrors)

cp new-layer-224.tl ferogram-tl-types/tl/api.tl
```

Make sure the first line of the file is:
```
// LAYER 224
```

## Step 2: Build

```bash
cargo build 2>&1 | head -40
```

The build script automatically:
- Parses the new schema
- Generates updated Rust source
- Patches `pub const LAYER: i32 = 224;` into `generated_common.rs`

If there are no breaking type changes in `ferogram`, it compiles cleanly.

## Step 3: Fix compile errors

New layers commonly add fields to existing structs. These show up as errors like:

```
error[E0063]: missing field `my_new_field` in initializer of `types::SomeStruct`
```

Fix them by adding the field with a sensible default:

```rust
// Boolean flags → false
my_new_flag: false,

// Option<T> fields → None
my_new_option: None,

// i32/i64 counts → 0
my_new_count: 0,

// String fields → String::new()
my_new_string: String::new(),
```

New enum variants in `match` statements:

```rust
// error[E0004]: non-exhaustive patterns: `Update::NewVariant(_)` not covered
Update::NewVariant(_) => { /* handle or ignore */ }
// OR add to the catch-all:
_ => {}
```

## Step 4: Bump version and publish

```bash
# In Cargo.toml workspace section
version = "0.4.8"
```

Then publish in dependency order (see [Publishing](../installation.md)).

## What propagates automatically

Once `api.tl` is updated with the new layer number, these update with **zero additional changes**:

| What | Where | How |
|---|---|---|
| `tl::LAYER` constant | `ferogram-tl-types/src/lib.rs` | build.rs patches it |
| `invokeWithLayer` call | `ferogram/src/lib.rs:1847` | reads `tl::LAYER` |
| `/about` bot command | `layer-bot/src/main.rs:333` | reads `tl::LAYER` at runtime |
| Badge in README | Manual: update once | String replace |

## Diff the changes

```bash
diff old-api.tl ferogram-tl-types/tl/api.tl | grep "^[<>]" | head -40
```

This shows you exactly which constructors changed, helping you anticipate which `ferogram` files need updating.
