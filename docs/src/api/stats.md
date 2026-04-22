# Stats & Analytics

ferogram exposes Telegram's channel and supergroup statistics endpoints.

> **Note**: Statistics are only available for channels with at least 500 subscribers and for supergroups that meet Telegram's threshold. The statistics DC must be reachable.

---

## Broadcast channel statistics

```rust
let stats = client.get_broadcast_stats("@mychannel", false).await?;
// dark=true requests dark-themed graph images
```

Returns `tl::enums::stats::BroadcastStats`. Key fields:

```rust
let tl::enums::stats::BroadcastStats::BroadcastStats(s) = stats;

// Follower count range
println!("Followers: {} (prev {})", s.followers.current, s.followers.previous);

// Views per post
println!("Views/post: {} (prev {})", s.views_per_post.current, s.views_per_post.previous);

// Shares per post
println!("Shares/post: {} (prev {})", s.shares_per_post.current, s.shares_per_post.previous);
```

`BroadcastStats` also contains graph references (`s.growth_graph`, `s.followers_graph`, `s.top_hours_graph`, `s.interactions_graph`, `s.iv_interactions_graph`, `s.views_by_source_graph`, `s.new_followers_by_source_graph`, `s.languages_graph`) that can be fetched via `stats.loadAsyncGraph` if they carry an async token.

---

## Supergroup (megagroup) statistics

```rust
let stats = client.get_megagroup_stats("@mysupergroup", false).await?;
```

Returns `tl::enums::stats::MegagroupStats`. Key fields:

```rust
let tl::enums::stats::MegagroupStats::MegagroupStats(s) = stats;

println!("Members: {} (prev {})", s.members.current, s.members.previous);
println!("Messages: {} (prev {})", s.messages.current, s.messages.previous);
println!("Viewers: {} (prev {})", s.viewers.current, s.viewers.previous);
println!("Posters: {} (prev {})", s.posters.current, s.posters.previous);
```

`MegagroupStats` also exposes `s.top_posters`, `s.top_admins`, `s.top_inviters` (lists of users with message/invite counts) and graph references for member growth, languages, messages by hour/weekday.

---

## Online member count

Get the current approximate number of online members in a group or channel:

```rust
let online = client.get_online_count("@mychannel").await?;
println!("{online} members are online right now");
```
