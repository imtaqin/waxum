# Dependency policy

waxum is a gateway, not a protocol implementation. The WhatsApp Web
protocol layer — handshake, Noise transport, Signal-protocol session
handling, binary node encoding, app-state sync — comes from
[`whatsapp-rust`](https://github.com/oxidezap/whatsapp-rust), a
third-party open-source project we do not control.

This document states what we depend on, how we pin it, when we move the
pin, and what that means for you if you depend on waxum.

## What we pin

Eight crates, all from the same upstream repository, all pinned to one
git revision:

| Crate | On crates.io |
|---|---|
| `whatsapp-rust` | 0.7.0 |
| `wacore` | 0.7.0 |
| `wacore-binary` | 0.7.0 |
| `waproto` | 0.7.0 |
| `whatsapp-rust-sqlite-storage` | 0.7.0 |
| `whatsapp-rust-tokio-transport` | 0.7.0 |
| `whatsapp-rust-ureq-http-client` | 0.7.0 |
| `whatsapp-rust-chat-store` | **not published** |

The current revision is recorded in `Cargo.toml`; the exact resolved
commit is in `Cargo.lock`.

## Why a git pin and not crates.io

Seven of the eight crates are published. `whatsapp-rust-chat-store` is
not, and waxum uses it for chat/message persistence.

The eight crates are not independent: they share types across their
public APIs. A `wacore` pulled from crates.io and a `wacore` pulled from
git are, to the compiler, two different crates with two different sets of
types — passing a value from one into the other does not compile. So the
whole set has to resolve from the same source. One unpublished crate
therefore forces all eight onto git.

A secondary reason: the pinned revision is usually ahead of the last
crates.io release, so the git pin is also where bug fixes land first.

## What this costs, and what it means for you

Be aware of this before you build on waxum:

- **waxum cannot be published to crates.io.** Cargo refuses to publish a
  crate with git dependencies. If you want waxum as a library dependency,
  you must depend on it by git too, and inherit the same constraint. As a
  deployed binary — the normal way to run waxum — this does not affect
  you at all.
- **No semver on the protocol layer.** A git revision carries no
  compatibility promise. Moving the pin can break the build or change
  behaviour with no version number signalling it. This is why the pin
  moves on a schedule (below) rather than opportunistically.
- **Weaker advisory coverage.** `cargo audit` matches RustSec advisories
  and yanks against crates.io versions. Git-pinned crates are matched far
  less reliably, so an advisory affecting the protocol layer may not
  surface in our CI audit. This is the one that most deserves attention.
- **Bus factor.** `whatsapp-rust` is a third-party project. If it stops
  being maintained, waxum's protocol layer stops receiving fixes, and
  taking that over means maintaining a WhatsApp protocol implementation
  in Rust — a materially different project from maintaining a REST
  gateway. We accept this risk knowingly rather than mitigate it: a fork
  costs more than it buys today.

We are not going to fork or vendor `whatsapp-rust` to escape any of the
above. The upstream project is healthy and moves faster than we would.
Making the exposure legible is the mitigation.

## Toolchain

`rust-toolchain.toml` pins `nightly-2026-04-05`. `rustup` honours that
file automatically, so contributors get the right toolchain without
choosing it.

The reason for nightly was upstream's use of `portable_simd`, an unstable
feature. That reason no longer holds at the revision we pin: upstream has
removed SIMD from its tree, declares a stable MSRV, and gates no unstable
features. waxum's own sources declare no `#![feature]` gates either.

On the available evidence the pin is probably removable — but "probably"
is not "verified", and nobody has run a stable build end to end. Until
someone does, the pin stays and this is the honest description of it.
Tracked in [#87](https://github.com/imtaqin/waxum/issues/87).

Removing the pin is not a one-line change to `rust-toolchain.toml`. It
needs a stable build of the full dependency graph — the eight upstream
crates included, since they are what forced nightly in the first place —
plus the quality gates and a live pairing smoke test, on the pinned
revision. Until that has been done once, treat `cargo +stable build` as
unsupported rather than broken; the distinction is that we have no
evidence either way. `error[E0554]` from an upstream crate is the
historical failure signature to look for.

Note also that the pin and the upstream revision are coupled. Bumping the
revision (below) can reintroduce an unstable feature upstream and make
nightly load-bearing again, which is one more reason the bump is a
deliberate per-release decision rather than a background upgrade.

## Bump cadence

**The upstream revision is reviewed once per release.** Not on a cron, not
continuously — it is a checklist item in the release process
(see [CONTRIBUTING.md](../CONTRIBUTING.md#releasing)), which makes it a
decision someone makes deliberately rather than a background upgrade that
lands unreviewed.

Reviewing does not mean always bumping. Staying put is a valid outcome;
what is not valid is not looking. At each release:

1. Compare the pinned revision against upstream `main`.
2. Read the intervening changes for protocol or API breaks.
3. If bumping: update all eight revisions together — they must stay on
   one revision — then run the full quality gates and a real pairing +
   send smoke test against a live session. Protocol regressions do not
   show up in `cargo test`.
4. Record the bump, and the reason, in `CHANGELOG.md`.
5. If not bumping, say so in the release notes so the next person knows
   the decision was made rather than missed.

Check whether the unpublished crate has been published at the same time.
If all eight land on crates.io, most of this document stops applying and
we should move to version requirements.
