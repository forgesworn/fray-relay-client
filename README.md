# fray-relay-client

A small Nostr relay client in Rust. It asks a set of relays for stored events
and gives back the verified ones within a time limit, keeps standing
subscriptions open across reconnects, publishes events and reports which
relays acknowledged them, and can answer NIP-42 authentication challenges.
Relay order, health and backoff come from the `fray-relays` crate.

## What it offers

- `fetch_live(urls, filters, timeout)`: a one-shot read for a caller with no
  runtime of its own. It returns a `Task<FetchReport>`.
- `fetch`, `fetch_report` and `fetch_first` take any `Relay`. They return the
  stored events matching the filters, deduplicated by id and newest first.
  A read returns when every relay has sent EOSE or failed, or at the timeout.
  `fetch_first` returns at the first event the caller accepts.
- `FetchReport` counts the relays asked (`total`) and the relays that answered
  (`answered`). "Nothing published" (`reached()` is true, no events) is kept
  apart from "could not reach the relays" (`reached()` is false).
- The `Relay` trait covers publishing, subscriptions, the URL list and a
  `reconnect` to dial again at once. `Subscription` yields `RelayMsg::Event` and
  one `RelayMsg::Eose` per relay. Its `SubStatus` is `Connecting`, `Up`, `Down`
  with a reason, or `Closed` with the relay's own words.
- `publish_acked` returns a `PublishAck`. Its `PublishReport` counts OK and
  refused answers. A write has succeeded once one relay answers OK.
- The `Connector` trait turns a list of URLs into a `Relay`. `WsConnector`
  runs one websocket worker per URL. `AuthConnector` does the same and also
  answers NIP-42 challenges with a signer (`auth_signer(keys)`).
  `PoolConnector` uses a `RelayPool` you supply. `WsRelaySet` is the live set
  behind all three.

## What the code enforces

- Every event a relay sends is checked against the subscription's filters and
  for a valid id and signature before it is queued. Events that fail are
  dropped.
- A websocket message or frame larger than 512 KiB is refused.
- Before its EOSE, a relay may send at most the sum of the filters' `limit`s
  (when every filter has one). Anything past that is dropped without being
  verified.
- Each subscription's queue holds at most 1024 messages (`SUB_QUEUE`), so a
  flooding relay's messages are dropped and the queue does not grow. Each relay
  has its own share of 256 slots in front of that queue. One relay can fill
  only its own share, and the last slot of each share is kept for that relay's
  EOSE. The tests check that a flooding relay neither crowds out another
  relay's answer nor ends a read before a slower relay has answered.
- A dial times out after 6 seconds. A relay that cannot be reached still ends
  its part of a read with an EOSE, but it is not counted in `answered`. A relay
  that is backing off is dialled again only when the pool allows it.
- Writes go to the top healthy relays in the given order, or to all of them
  when none is healthy. Kind 24133
  (NIP-46) events go to every listed relay and never to fallback relays. A
  write to a relay that is down waits in a queue of at most 32 events per
  relay, for up to 120 seconds.
- NIP-42: a challenge is answered only through `AuthConnector`, at most 3
  times per connection. A challenge is answered only if it is non-empty
  printable ASCII of at most 256 bytes. A request closed `auth-required` is
  asked again once the relay accepts the answer.
- Text a relay supplies for a `SubStatus` or a failure, such as a CLOSED
  reason, has control, bidi and zero-width characters removed. The reason a
  relay gives for refusing an authentication is logged with control
  characters removed.
- The crate's log lines use the target `fray_relay_client`.

The layer is a pipe, not a gate. Events that pass these checks are still
untrusted data until the code above has checked what they claim.

## Synchronous and non-blocking

The crate starts its own two-thread tokio runtime on first use and runs every
websocket worker on it; `runtime()` returns it, so that a layer above can run
its work on the same runtime. That work must not block: the runtime's two
threads also carry every relay connection. The connectors and the `Relay` methods
`publish`, `publish_acked` and `subscribe` are plain functions that return at
once, and `Subscription::try_recv` never waits. `fetch_live` needs no runtime
from the caller: it returns a `Task<T>`, whose `poll()` gives the result once,
without blocking, which suits a frame loop; `wait(timeout)` blocks and is meant
for tools and tests. `fetch`, `fetch_report`, `fetch_first`,
`Subscription::recv` and `PublishAck::wait` are `async`, for code that already
runs on a runtime (`Task::spawn` puts such code on this crate's).

## An in-memory fake for tests

`FakeNetwork` is a `Connector` backed by one in-memory `FakeRelayHub` per URL.
A hub accepts only events with a valid signature, stores regular events,
keeps the newest replaceable and addressable version, delivers ephemeral
events to subscribers without storing them, and handles NIP-09 deletion and
NIP-40 expiration. It can be switched offline, failing,
closing, mute or refusing, and it records the requests it has seen. The fake is
ordinary public code, so the tests of crates that use this one can run without
a network.

## Relay health: `fray-relays`, pinned

Relay order, health, backoff and the fallback to default relays live in
`RelayPool` from [`fray-relays`](https://github.com/forgesworn/fray). It is a
git dependency at a fixed revision, under the local name `decented-relays`:

```toml
decented-relays = { package = "fray-relays", git = "https://github.com/forgesworn/fray", rev = "4943413bb86a329c3ad9cc3180c02fd9186ef2ce" }
```

`RelayPool::global()` is one pool for the whole process. Two crates in the
same build share it, and so share relay health, only when both resolve the
same `fray-relays` revision. With different revisions, Cargo builds two
separate crates with two separate pools. A crate that uses this one and also
depends on `fray-relays` should pin the same revision.

The same holds for this crate. A crate that depends on both this crate and a
crate that re-exports it should pin the same revision of this crate. With
different revisions or sources, the build holds two copies: two runtimes, so
four worker threads, and two incompatible `Task` types.

## Tests

```
cargo test
```

The tests run against the in-memory fake and against websocket servers on
127.0.0.1. They need no internet. One test reads from public relays over TLS
and is ignored by default:

```
cargo test --test ws -- --ignored
```

`tests/no_game_names.rs` fails if a game name appears in library code.

## Licence

MIT. See `LICENSE`. `NOTICE` says where the code came from.
