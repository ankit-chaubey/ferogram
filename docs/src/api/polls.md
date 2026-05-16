# Polls & Votes

ferogram lets you send polls, vote on them, and inspect results and voter lists.

---

## Sending a poll

Use `PollBuilder` and pass it to `send_poll`:

```rust
use ferogram::{Client, PollBuilder};

// Regular anonymous poll
client.send_poll(peer.clone(),
    PollBuilder::new("What is your favourite language?")
        .answers(["Rust", "Go", "C++"])
).await?;

// Public voters, auto-close after 5 minutes
client.send_poll(peer.clone(),
    PollBuilder::new("Vote now")
        .answers(["Yes", "No"])
        .public_voters(true)
        .close_period(300)
).await?;

// Quiz mode with explanation
client.send_poll(peer.clone(),
    PollBuilder::new("Capital of France?")
        .answers(["Berlin", "Paris", "Rome"])
        .quiz(true)
        .correct_index(1)
        .solution("It's Paris.")
        .hide_results_until_close(true)
).await?;

// Multiple choice
client.send_poll(peer.clone(),
    PollBuilder::new("Pick your tools")
        .answers(["vim", "emacs", "VSCode", "Helix"])
        .multiple_choice(true)
).await?;
```

### `PollBuilder` methods

| Method | Description |
|---|---|
| `PollBuilder::new(question)` | Start a builder with a question string |
| `.answers(iter)` | Answer strings, in order |
| `.quiz(bool)` | Quiz mode (one correct answer) |
| `.correct_index(usize)` | Which answer index is correct (quiz only) |
| `.solution(text)` | Explanation shown after quiz answer |
| `.multiple_choice(bool)` | Allow selecting more than one answer |
| `.public_voters(bool)` | Show who voted |
| `.shuffle_answers(bool)` | Randomise answer order per viewer |
| `.hide_results_until_close(bool)` | Keep results hidden until closed |
| `.close_period(secs: i32)` | Auto-close after N seconds (1-600) |
| `.close_date(ts: i32)` | Auto-close at Unix timestamp |
| `.subscribers_only(bool)` | Only channel subscribers can vote |
| `.countries_iso2(codes)` | Restrict voting to ISO 3166-1 alpha-2 country codes |

---

## Voting

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.send_vote(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, options: Vec&lt;Vec&lt;u8&gt;&gt;) -> Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Cast a vote. `options` is a list of answer byte vectors matching the `option` field on each `PollAnswer`. For single-choice polls pass one item; for multiple-choice pass several.

```rust
// Vote for option 0 on message 1234
client.send_vote(peer.clone(), 1234, vec![vec![0]]).await?;
```
</div>
</div>

---

## Results

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.poll_results(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, poll_hash: i64) -> Result&lt;(), InvocationError&gt;</span>
</div>
<div class="api-card-body">
Request a fresh result snapshot from the server. The server responds with an `updateMessagePoll` in the update stream. `poll_hash` comes from `PollResults.results_hash` on the message.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.get_poll_votes(peer: impl Into&lt;PeerRef&gt;, msg_id: i32, option: Option&lt;Vec&lt;u8&gt;&gt;, limit: i32, offset: Option&lt;String&gt;) -> Result&lt;tl::types::messages::VotesList, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Paginated list of who voted. Only works for public-voters polls.

- `option`: filter to a specific answer byte vector, or `None` for all votes.
- `offset`: continuation token from the previous page's `next_offset`.

```rust
let page = client
    .get_poll_votes(peer.clone(), msg_id, Some(vec![1]), 50, None)
    .await?;

for vote in &page.votes {
    println!("User {} voted at {}", vote.user_id, vote.date);
}

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

## Poll stats

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">client.poll_results(peer: impl Into&lt;PeerRef&gt;, msg_id: i32) -> Result&lt;tl::types::stats::PollStats, InvocationError&gt;</span>
</div>
<div class="api-card-body">
Fetch detailed vote graph stats for a poll (`stats.getPollStats`). Returns a `PollStats` with a `votes_graph` field containing a `StatsGraph` you can render as a chart.

```rust
let stats = client.poll_results(peer.clone(), msg_id).await?;
// stats.votes_graph: tl::enums::StatsGraph
```
</div>
</div>

---

## Closing a poll

Edit the message and set `closed: true` on the poll. You need the original poll ID from the message:

```rust
let close_media = tl::enums::InputMedia::Poll(Box::new(tl::types::InputMediaPoll {
    poll: tl::enums::Poll::Poll(tl::types::Poll {
        id: existing_poll_id,
        closed: true,
        // other fields same as original
        ..
    }),
    correct_answers: None,
    solution: None,
    solution_entities: None,
    solution_media: None,
}));

client.invoke(&tl::functions::messages::EditMessage {
    peer: input_peer,
    id: msg_id,
    media: Some(close_media),
    // ..
}).await?;
```
