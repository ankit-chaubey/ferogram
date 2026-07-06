# The Highway Model: How Ferogram Moves Files

Ferogram's file transfer engine is built around one idea, borrowed straight
from Telegram's own file API documentation: a transfer goes faster when you
overlap network round trips instead of waiting on them one at a time. This
page explains the model, why it's shaped the way it is, and every knob you
can turn.

## Why this exists

Telegram's file API docs describe two independent ways to speed up an
upload or download:

1. Open more than one connection for a single transfer.
2. Keep more than one chunk request in flight per connection, instead of
   waiting for each response before sending the next.

Most MTProto clients only do the first one. Ferogram does both, and makes
both configurable, because the two techniques solve different problems and
cost different things. Bolting them together into one "concurrency" number
would hide that distinction and make it impossible to tune correctly.

## The model

Picture a file transfer as trucks travelling on highways.

- **A truck** is one in-flight chunk request: `upload.saveFilePart`,
  `upload.saveBigFilePart`, or `upload.getFile`.
- **The load a truck carries** is the chunk size: how many bytes one
  request moves. Decided purely by file size, not by anything you
  configure (more on this below).
- **The length of the highway** is round-trip time (RTT): how long one
  truck takes to reach Telegram's datacenter and come back with a
  response. Set by the network path between you and the DC. You cannot
  shorten the road. What you can do is put more trucks on it at once.
- **A highway** is one dedicated MTProto connection (TCP socket) opened
  just for this transfer.
- **Y** is how many highways a single transfer is allowed to build for
  itself.
- **X** is how many trucks are allowed on one highway at once.
- The **global cap** is a city-wide speed limit on total highways: every
  transfer's highways, upload and download alike, draw from one shared
  pool.

Put together:

```
total trucks in flight for one transfer = Y × X
total bytes in flight                   = Y × X × chunk size
```

A transfer using 2 highways at a pipeline depth of 4 can have up to 8
chunks moving at once, using only 2 sockets.

## Why Y and X are separate knobs, not one

This is the part that's easy to collapse into a single "concurrency"
setting, and exactly why Ferogram doesn't.

| | Y (connections) | X (pipeline depth) |
|---|---|---|
| What it costs | A real socket and MTProto session on **Telegram's side** | A chunk buffer in **your process's memory** |
| What happens if you push too far | Telegram sheds the connection with an early-EOF | Nothing breaks, you just use more RAM |
| Hard ceiling | 4 per file (`MAX_WORKERS_PER_FILE`) | 8 per connection (`MAX_PIPELINE_DEPTH`) |
| Helps most when | Bandwidth is the bottleneck | Round-trip time is the bottleneck |

Because Y costs the server something and X only costs you memory, they
need different ceilings and different levels of caution. A single merged
knob would either be too conservative on the axis that's actually free
(X), or too permissive on the axis that gets your connections shed (Y).

## What's configurable

Set through [`TransferLimits`](https://docs.rs/ferogram), either via
`ClientBuilder` shorthands or as one struct:

```rust
Client::builder()
    .download_tcp_connections(2)   // Y for downloads
    .upload_tcp_connections(4)     // Y for uploads
    .max_tcp_connections(6)        // shared global cap
    .download_pipeline_depth(2)    // X for downloads
    .upload_pipeline_depth(6)      // X for uploads
    .connect().await?;
```

| Field | Meaning | Default | Hard ceiling |
|---|---|---|---|
| `download_tcp_connections` | Y, downloads only | 4 | 4 |
| `upload_tcp_connections` | Y, uploads only | 4 | 4 |
| `max_tcp_connections` | shared pool, both directions | 12 | none (min 1) |
| `download_pipeline_depth` | X, downloads only | 4 | 8 |
| `upload_pipeline_depth` | X, uploads only | 4 | 8 |

Download and upload each get their own Y and X because a link's upload
and download bandwidth are frequently different, and there's no reason to
force one number to describe both directions.

Every field is clamped to its ceiling on connect (`ClientBuilder::build`
and again in `ClientInner`, so it can't be bypassed even by hand-building
a `Config`). Whatever you ask for, the library never lets you exceed the
number Ankit has already verified is safe.

## Normally, Y comes from a table, not a fixed number

Y isn't just "whatever you set" - by default it's looked up from file
size, and your configured value acts as a ceiling on that lookup rather
than a fixed count:

**Downloads:**

| File size | Y (before your ceiling applies) |
|---|---|
| < 10 MB | 1 |
| 10 - 50 MB | 2 |
| 50 - 300 MB | 3 |
| > 300 MB | up to your ceiling (max 4) |

**Uploads:**

| File size | Y (before your ceiling applies) |
|---|---|
| < 10 MB | 1 |
| 10 - 100 MB | 2 |
| 100 - 500 MB | 3 |
| > 500 MB | up to your ceiling (max 4) |

A 5 MB file never opens more than one connection, no matter how high you
set the ceiling. The table exists so small, fast transfers don't pay the
overhead of extra sockets they don't need.

X does not have a table. Every worker connection gets the same
configured pipeline depth regardless of file size, because unlike Y it
doesn't cost the server anything extra to keep more requests queued on a
connection that's already open.

## bypass_tcp_allotments: skipping the table

```rust
.bypass_tcp_allotments(true)
```

This removes the size-based Y lookup entirely. Every transfer in that
direction opens exactly your configured `download_tcp_connections` /
`upload_tcp_connections`, even a tiny file that would normally only need
one connection.

This is an override, not a tuning knob, and it changes real load Telegram
wasn't expecting for that file size. See the responsibility section
below before turning it on.

## What's deliberately not configurable

**Chunk size** - decided purely by file size, mirroring Telegram's own
official client constants:

Downloads (`download_chunk_size`):

| File size | Chunk |
|---|---|
| < 50 MB | 256 KB |
| 50 - 500 MB | 512 KB |
| > 500 MB | 1 MB (offsets stay 1 MB-aligned, satisfying `GetFile`'s alignment rule) |

Uploads (`upload_part_size`), five tiers from 32 KB up to 512 KB, with a
built-in safety clamp: if a chosen part size would ever push the part
count past Telegram's 4000-part ceiling, the part size grows (in 512-byte
increments) until it fits.

Chunk size isn't part of `TransferLimits` because getting it wrong risks
an outright invalid request (bad offset alignment), not just a slower
transfer. It also silently multiplies against whatever Y and X you've
set (`Y × X × chunk size` = memory in flight), so it's exactly the kind
of number that shouldn't be tunable independently of everything else that
already assumes specific values for it.

**Two thresholds, not one** - routing decisions about *when* a transfer
bothers using the concurrent/pipelined engine at all are also fixed, and
deliberately split by direction:

- `BIG_FILE_THRESHOLD` (10 MB) - upload-only. This is Telegram's actual
  protocol boundary between `upload.saveFilePart` and
  `upload.saveBigFilePart`.
- `DOWNLOAD_CONCURRENT_THRESHOLD` (10 MB) - download-only. `GetFile` has
  no small/big file distinction in the protocol; this is purely a
  routing choice for when Ferogram bothers opening multiple connections
  for a download. It used to share a constant with the upload threshold,
  which was coincidental, not protocol-driven, so the two were split
  apart even though they currently hold the same value.

## Overriding defaults is your responsibility

Ferogram's defaults were tuned against how Telegram's servers actually
behave, not just what the protocol technically allows. Every field is
still clamped to a hard ceiling Ankit controls, so nothing you configure
can break the protocol outright. But within those ceilings, you have real
room to push harder than the defaults - and if you do, that's a trade
you're choosing to make, not one Ferogram is making for you.

The most common way this shows up is `FLOOD_WAIT`. Telegram enforces
per-account, per-DC rate limits that aren't published and can change
without notice. Pushing Y and X higher, or enabling
`bypass_tcp_allotments`, means more simultaneous requests hitting the same
account and DC. Ferogram already handles an individual `FLOOD_WAIT`
correctly on its own - it backs off for the requested duration and
retries just the affected chunk - but if you're seeing `FLOOD_WAIT`
regularly, that's Telegram telling you directly that your configured
concurrency is higher than it's currently willing to tolerate.

**If that happens: turn the dial back down, in this order.**

1. Reduce X first (pipeline depth): it's the cheaper adjustment and
   doesn't change how many sockets you're holding open.
2. Reduce Y next (connections per file) if flood waits continue.
3. Turn off `bypass_tcp_allotments` if it was on, so small files go back
   to using only as many connections as they actually need.

There's no configuration that makes Telegram's rate limits disappear;
only one that respects them.

## Current status of the defaults

The default values (`workers_per_file = 4`, `pipeline_depth = 4`,
`max_pipeline_depth = 8`) are initial estimates, not the result of
extensive field testing across many devices and networks. They're
expected to be revisited as real usage feedback comes in. If you find a
default that's clearly wrong for common conditions, that's useful
information, not just a workaround to route around quietly.
