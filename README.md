# framed-echo

A small, production-quality demonstration of a **zero-copy, length-prefixed
framing protocol** over [Tokio](https://tokio.rs/): a codec, an async echo
server, and a client — with tests and benchmarks as first-class citizens.

It exists to show two mechanics clearly and correctly:

1. **Framing under partial reads** — turning TCP's boundary-less byte stream
   back into discrete messages.
2. **Zero-copy payloads** — handing decoded messages to the application without
   a per-message allocation or copy.

Both are commented inline where they happen, in
[`src/codec.rs`](src/codec.rs).

### Interactive walkthrough

An interactive, browser-based walkthrough lives in [`docs/`](docs/index.html):
drive bytes into the receive buffer the way a real socket would, and watch each
`decode()` call decide — partial prefix, partial payload, oversized rejection,
and the zero-copy handoff — with a live trace. It runs the same four-guard logic
as the codec.

When this repo is pushed to GitHub with Pages enabled (**Settings → Pages →
Source: "Deploy from a branch" → `main` / `docs`**), it is served at
<https://1-bit-wonder.github.io/framed-echo/>. It is a single self-contained
HTML file with no build step and no external assets.

---

## Plain-language explainer

*Written for a technical reader who hasn't dealt with framing internals before.*

### The problem: TCP has no message boundaries

TCP gives you an ordered, reliable **stream of bytes** — and nothing more.
Crucially, it does **not** preserve *message* boundaries. If you write three
messages of 10 bytes each, the receiver might read them as one 30-byte chunk,
or as 7 + 23, or one byte at a time. TCP guarantees the bytes arrive **in
order**, not that they arrive **grouped the way you sent them**.

So any application that sends discrete messages over TCP has to add its own
framing on top. The standard, boring, correct solution is **length-prefixing**:
before each message, write its length; the receiver reads the length first, then
knows exactly how many bytes make up the message that follows.

This project's wire format is the minimal version of that:

```
+------------------------+-------------------------------+
| length: u32 big-endian |   payload: exactly `length`   |
|        (4 bytes)        |             bytes             |
+------------------------+-------------------------------+
```

### Where this shows up in the real world

Length-prefixed framing over TCP is everywhere custom binary protocols live:

- **Databases** — the PostgreSQL, Redis (RESP), and Kafka wire protocols all
  frame their messages.
- **RPC systems and message brokers** — gRPC/HTTP2 frames, AMQP, MQTT.
- **Game servers** and **real-time communications** (VoIP / SIP / WebRTC media
  and signaling), which parse framed packets at high rates where per-packet
  overhead is felt directly.

The specifics differ, but the core move — *read a size, then read that many
bytes* — is the same.

### Hard part 1 — partial frames (correctness)

The subtle part isn't the happy path; it's that a single read can contain **half
a frame, exactly one, or several glued together**. A decoder that assumes "one
read = one message" works perfectly on localhost and then corrupts data the
moment it meets a real network that splits or coalesces reads. It's a classic
bug: passes local testing, fails in production.

The decoder here handles it explicitly. On each call it checks:

- Are all 4 length-prefix bytes present yet? If not, return `Ok(None)` — "no
  frame yet, call me again" — **without discarding what's buffered**.
- Is the full payload present yet? If not, again `Ok(None)`, and it
  `reserve()`s room for the missing bytes so the next read can land in one shot.

Only when a *complete* frame is buffered does it consume anything. Incomplete
data is left untouched in the buffer for the next call. This is the
`Ok(None)`-and-reserve path, and it's what makes the decoder correct under
arbitrary read fragmentation. The included test feeds a frame **one byte at a
time** and asserts the decoder yields nothing until the very last byte arrives.

### Hard part 2 — zero-copy (performance)

The naive decoder allocates a fresh `Vec<u8>` and **copies** the payload into it
for every message. At a few messages per second that's invisible; at hundreds of
thousands per second, that allocation-and-copy churn becomes a real latency and
throughput tax (allocator pressure, cache traffic, GC-like pauses in other
runtimes).

This project instead returns [`bytes::Bytes`](https://docs.rs/bytes) — a
**refcounted view into the receive buffer that already holds the bytes**. The
mechanism is two calls in the decoder:

- `BytesMut::split_to(payload_len)` detaches the payload region into its own
  handle that **shares the same underlying allocation** — a refcount bump, not a
  `memcpy`.
- `.freeze()` converts that handle into an immutable `Bytes` the application can
  hold.

There is **no per-frame allocation and no payload copy** on the decode path,
just pointer/refcount bookkeeping. The shared buffer is freed automatically when
the last `Bytes` view into it is dropped — no manual lifetime management.

### Why it matters for latency-sensitive backends

For real-time systems — voice, video, multiplayer, live signaling — per-packet
overhead is not an abstraction; it's what the user experiences as jitter or lag.
Cutting a heap allocation and a copy out of the hot path of *every* packet is
exactly the kind of overhead reduction that keeps tail latencies flat under
load.

### Honest scope

This is a **demonstration** — a deliberately small miniature of real systems.
Production protocols layer on authentication, version negotiation, compression,
flow control, backpressure, and multiplexing, none of which are here. What *is*
here are the two load-bearing mechanics that those larger systems still rely on
at their core: **correct framing under partial reads**, and **zero-copy
payloads**. Get these two right and you have the spine of a real wire protocol.

---

## Wire format

| Field     | Size     | Encoding                | Notes                                   |
| --------- | -------- | ----------------------- | --------------------------------------- |
| `length`  | 4 bytes  | big-endian `u32`        | number of payload bytes that follow     |
| `payload` | `length` | raw bytes               | opaque to the codec                     |

- Maximum payload size is configurable (default **8 MiB**). A frame declaring a
  larger payload is **rejected with an error before any payload is allocated**,
  so a corrupt or hostile length prefix can't trigger a huge allocation.
- A zero-length payload is a valid frame (prefix of `0`, no bytes).

---

## Project layout

```
src/
  lib.rs           crate docs + re-exports
  codec.rs         FrameCodec: Decoder + Encoder, error type, unit tests
  bin/server.rs    async echo server (task per connection)
  bin/client.rs    demo client: sends varied sizes, verifies echoes
tests/
  integration.rs   real server on 127.0.0.1:0 + real client over TCP
benches/
  codec.rs         criterion encode/decode throughput
flake.nix          reproducible dev shell + package build (Nix)
```

---

## Running it

The project pins its entire toolchain and dependency graph with **Nix flakes**,
so the commands below run identically on any machine with Nix. If you'd rather
use your own Rust toolchain, drop the `nix develop -c` prefix — a stable Rust
with the 2021 edition is all that's needed.

### Reproducible environment (Nix)

```sh
nix develop            # drop into a shell with the pinned rustc/cargo/clippy/etc.
# or, with nix-direnv:
direnv allow           # auto-loads the same shell on cd
```

`nix build` compiles the library and both binaries from the exact crate versions
in `Cargo.lock`:

```sh
nix build
./result/bin/framed-echo-server
```

### Server + client

```sh
# Terminal 1 — start the echo server (default 127.0.0.1:9000)
nix develop -c cargo run --bin framed-echo-server
# optional: pass an address, or set FRAMED_ECHO_ADDR
nix develop -c cargo run --bin framed-echo-server -- 127.0.0.1:9000

# Terminal 2 — run the client; it sends frames of many sizes and checks echoes
nix develop -c cargo run --bin framed-echo-client -- 127.0.0.1:9000
```

Expected client output:

```
frame 0: 0 bytes echoed correctly
frame 1: 1 bytes echoed correctly
frame 2: 16 bytes echoed correctly
frame 3: 1024 bytes echoed correctly
frame 4: 65536 bytes echoed correctly
frame 5: 1048576 bytes echoed correctly
all 6 frames verified
```

### Tests

```sh
nix develop -c cargo test
```

Covers: encode/decode round-trip, partial-frame handling (fed one byte at a
time), multiple frames in one buffer, empty payloads, oversized-frame rejection
(encode and decode), and two end-to-end integration tests over real TCP on an
ephemeral port.

### Benchmark

```sh
nix develop -c cargo bench
```

---

## Benchmark results

**Measured locally — not a portable claim.** These are the point (median)
estimates from one `cargo bench` run on an **AMD Ryzen 7 7700X**, Rust 1.95.0,
`release` profile. Your numbers will differ; reproduce them with the command
above. The benchmark measures the codec's `encode`/`decode` in isolation (no
sockets), so it reflects the framing + zero-copy path, not network I/O.

| Operation | Payload   | Time (median) | Throughput (median) |
| --------- | --------- | ------------- | ------------------- |
| encode    | 64 B      | 22.9 ns       | 2.60 GiB/s          |
| encode    | 1 KiB     | 31.3 ns       | 30.5 GiB/s          |
| encode    | 64 KiB    | 822.6 ns      | 74.2 GiB/s          |
| decode    | 64 B      | 26.4 ns       | 2.26 GiB/s          |
| decode    | 1 KiB     | 34.0 ns       | 28.0 GiB/s          |
| decode    | 64 KiB    | 814.7 ns      | 74.9 GiB/s          |

Note that decode time barely grows with payload size (26 ns → 815 ns is
dominated by the throughput measurement's buffer clone, not the frame handoff):
the zero-copy handoff itself is O(1) in the payload size — it's a refcount bump,
not a copy.

---

## Design constraints

- Idiomatic modern Rust, **edition 2021**, **no `unsafe`**.
- **No `.unwrap()` / `.expect()`** in the library or server paths; errors flow
  through a meaningful [`FrameError`](src/codec.rs) that implements
  `std::error::Error` and wraps I/O errors cleanly. (Tests and benches use
  `unwrap` freely, as is conventional.)
- Comments are concentrated where they earn their keep: the zero-copy handoff
  and the partial-frame logic.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
