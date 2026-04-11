# Reactions

Reactions are emoji responses attached to messages. They appear below messages with per-emoji counts.

`ferogram` provides the [`InputReactions`](https://docs.rs/ferogram/latest/ferogram/reactions/struct.InputReactions.html) builder. It converts from `&str` and `String` automatically, so simple cases need no import.

---

## Send a reaction

```rust
use ferogram::reactions::InputReactions;

// Standard emoji: shorthand (no import needed, &str converts automatically)
client.send_reaction(peer.clone(), message_id, "👍").await?;

// Standard emoji: explicit builder
client.send_reaction(peer.clone(), message_id, InputReactions::emoticon("🔥")).await?;

// Custom (premium) emoji by document_id
client.send_reaction(peer.clone(), message_id, InputReactions::custom_emoji(1234567890)).await?;

// Remove all reactions from the message
client.send_reaction(peer.clone(), message_id, InputReactions::remove()).await?;
```

---

## Modifiers

Chain modifiers after a constructor:

```rust
// Big animated reaction (full-screen effect)
client.send_reaction(peer.clone(), message_id,
    InputReactions::emoticon("🔥").big()
).await?;

// Add to the user's recently-used reaction list
client.send_reaction(peer.clone(), message_id,
    InputReactions::emoticon("❤️").add_to_recent()
).await?;

// Both at once
client.send_reaction(peer.clone(), message_id,
    InputReactions::emoticon("🎉").big().add_to_recent()
).await?;
```

---

## Multi-reaction (premium users)

```rust
use ferogram_tl_types::{enums::Reaction, types};

client.send_reaction(
    peer.clone(),
    message_id,
    InputReactions::from(vec![
        Reaction::Emoji(types::ReactionEmoji { emoticon: "👍".into() }),
        Reaction::Emoji(types::ReactionEmoji { emoticon: "❤️".into() }),
    ]),
).await?;
```

---

## `InputReactions` API reference

| Constructor | Description |
|---|---|
| `InputReactions::emoticon("👍")` | React with a standard Unicode emoji |
| `InputReactions::custom_emoji(doc_id)` | React with a custom (premium) emoji |
| `InputReactions::remove()` | Remove all reactions from the message |
| `InputReactions::from(vec![…])` | Multi-reaction from a `Vec<Reaction>` |

| Modifier | Description |
|---|---|
| `.big()` | Play the full-screen large animation |
| `.add_to_recent()` | Add to the user's recent reactions list |

`InputReactions` also implements `From<&str>` and `From<String>`, so you can pass a plain emoji string directly to `send_reaction`.

---

## Reading reactions from a message

Reactions are embedded in `msg.raw`:

```rust
Update::NewMessage(msg) => {
    if let tl::enums::Message::Message(m) = &msg.raw {
        if let Some(tl::enums::MessageReactions::MessageReactions(r)) = &m.reactions {
            for result in &r.results {
                if let tl::enums::ReactionCount::ReactionCount(rc) = result {
                    match &rc.reaction {
                        tl::enums::Reaction::Emoji(e) => {
                            println!("{}: {} users", e.emoticon, rc.count);
                        }
                        tl::enums::Reaction::CustomEmoji(e) => {
                            println!("custom {}: {} users", e.document_id, rc.count);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
```

---

## Raw API: who reacted

To see which users chose a specific reaction:

```rust
use ferogram_tl_types::{functions, enums, types};

let result = client.invoke(&functions::messages::GetMessageReactionsList {
    peer:     peer_input,
    id:       message_id,
    reaction: Some(enums::Reaction::Emoji(types::ReactionEmoji {
        emoticon: "👍".into(),
    })),
    offset:   None,
    limit:    50,
}).await?;

if let enums::messages::MessageReactionsList::MessageReactionsList(list) = result {
    for r in &list.reactions {
        if let enums::MessagePeerReaction::MessagePeerReaction(rr) = r {
            println!("peer: {:?}", rr.peer_id);
        }
    }
}
```
