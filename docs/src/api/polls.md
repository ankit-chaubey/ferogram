# Polls & Votes

ferogram lets you send polls via `InputMessage`, vote on them programmatically, and inspect results and per-option vote lists.

---

## Sending a poll

Polls are sent as media via `InputMessage`. Build the poll TL object and attach it:

```rust
use ferogram::InputMessage;

let poll = tl::enums::InputMedia::InputMediaPoll(tl::types::InputMediaPoll {
    poll: tl::enums::Poll::Poll(tl::types::Poll {
        id: 0,
        closed: false,
        public_voters: false,  // anonymous
        multiple_choice: false,
        quiz: false,
        question: tl::enums::TextWithEntities::TextWithEntities(
            tl::types::TextWithEntities { text: "What is your favourite language?".into(), entities: vec![] }
        ),
        answers: vec![
            tl::enums::PollAnswer::PollAnswer(tl::types::PollAnswer {
                text: tl::enums::TextWithEntities::TextWithEntities(
                    tl::types::TextWithEntities { text: "Rust".into(), entities: vec![] }
                ),
                option: vec![0],
            }),
            tl::enums::PollAnswer::PollAnswer(tl::types::PollAnswer {
                text: tl::enums::TextWithEntities::TextWithEntities(
                    tl::types::TextWithEntities { text: "Go".into(), entities: vec![] }
                ),
                option: vec![1],
            }),
        ],
        close_period: None,
        close_date: None,
    }),
    correct_answers: None,
    solution: None,
    solution_entities: None,
});

client.send_message(peer.clone(), InputMessage::text("").copy_media(poll)).await?;
```

---

## Voting

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_vote(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, options: Vec&lt;Vec&lt;u8&gt;&gt;) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Cast a vote on a poll message. <code>options</code> is a list of answer byte arrays  -  each matches the <code>option</code> field of a <code>PollAnswer</code>. For single-choice polls pass exactly one option; for multiple-choice polls pass multiple.

```rust
// Vote for option 0 (Rust) on message 1234
client.send_vote(peer.clone(), 1234, vec![vec![0]]).await?;
```
</div>
</div>

---

## Results

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_poll_results(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, poll_hash: i64) → Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Request a fresh result snapshot from the server. The server responds with an <code>updateMessagePoll</code> in the update stream  -  this method itself returns <code>()</code> after the request is sent. The <code>poll_hash</code> comes from the <code>PollResults.results_hash</code> field on the message.

Use this to force-refresh results that may be stale in your local cache.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_poll_votes(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, option: Option&lt;Vec&lt;u8&gt;&gt;, limit: i32, offset: Option&lt;String&gt;) → Result&lt;tl::types::messages::VotesList, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch the list of users who voted, paginated. Only available for <strong>public voters</strong> polls (<code>public_voters: true</code>).

- `option`  -  filter to a specific answer byte vector, or `None` for all votes.
- `offset`  -  continuation token from a previous page's `next_offset` field.

```rust
// First page: who voted for option 1?
let page = client
    .get_poll_votes(peer.clone(), msg_id, Some(vec![1]), 50, None)
    .await?;

for vote in &page.votes {
    println!("User {} voted at {}", vote.user_id, vote.date);
}

// Next page
if let Some(next) = page.next_offset {
    let page2 = client
        .get_poll_votes(peer.clone(), msg_id, Some(vec![1]), 50, Some(next))
        .await?;
}
```

### `VotesList` fields

| Field | Type | Description |
|---|---|---|
| `count` | `i32` | Total vote count across all options |
| `votes` | `Vec<MessageUserVote>` | Votes for this page |
| `users` | `Vec<User>` | Resolved user objects for `votes` |
| `next_offset` | `Option<String>` | Pagination cursor for the next page |
</div>
</div>

---

## Closing a poll

To close a poll (stop accepting votes), edit the message and set `closed: true` on the poll:

```rust
// Get the current poll from the message, then re-send with closed=true
// You must reconstruct the InputMediaPoll with the same poll ID and closed=true.
let close_media = tl::enums::InputMedia::InputMediaPoll(tl::types::InputMediaPoll {
    poll: tl::enums::Poll::Poll(tl::types::Poll {
        id: existing_poll_id,
        closed: true,  // <-- close it
        // ... same options etc.
        # public_voters: false, multiple_choice: false, quiz: false,
        # question: tl::enums::TextWithEntities::TextWithEntities(tl::types::TextWithEntities { text: "".into(), entities: vec![] }),
        # answers: vec![],
        # close_period: None, close_date: None,
    }),
    correct_answers: None,
    solution: None,
    solution_entities: None,
});

client.invoke(&tl::functions::messages::EditMessage {
    peer: input_peer,
    id: msg_id,
    media: Some(close_media),
    // ...
}).await?;
```
