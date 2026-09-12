<p align="center">
  <img src="https://waxum.imtaqin.id/img/logo.png" alt="Waxum" width="140" />
</p>

<h1 align="center">Waxum</h1>

<p align="center">
  WhatsApp REST API gateway. Written in Rust.
</p>

<p align="center">
  <a href="https://waxum.imtaqin.id">Docs</a> ·
  <a href="https://waxum.imtaqin.id/docs/api/sessions">API</a> ·
  <a href="https://github.com/imtaqin/waxum/releases">Releases</a>
</p>

---

Native single-binary. Multi-session. Multi-DB. Webhooks + HMAC. JWT + Bearer. Swagger. Prometheus. NATS JetStream (optional).

Production-grade. **180+ REST endpoints across 29 feature modules.**

## Features

### Messaging

| Feature | Endpoint prefix |
|---|---|
| Text, image, video, audio, document, sticker | `POST /sessions/{sid}/messages/*` |
| Location, contact card, poll | `POST /sessions/{sid}/messages/{location,contact,poll}` |
| Reactions, forward, edit, revoke, star | `POST /sessions/{sid}/messages/{react,forward,edit,delete,star}` |
| Reply / quote with context | `POST /sessions/{sid}/messages/text` (`quoted_id`) |
| Fake-reply (spoofed quote metadata) | `POST /sessions/{sid}/fake-reply/*` |
| CTA URL with image + header/footer | `POST /sessions/{sid}/messages/cta-url` |
| Broadcast lists | `POST /sessions/{sid}/broadcast` |
| Chat state (typing / recording / paused) | `POST /sessions/{sid}/chat-state` |
| Read receipts, mark as read | `POST /sessions/{sid}/messages/read` |
| MEX GraphQL passthrough (server queries) | `POST /sessions/{sid}/mex` |
| Message history + full-text search (SQLite FTS5) | `GET /sessions/{sid}/messages/search?q=`, `GET /messages/search?q=` |

### Scheduling & bulk send

| Feature | Endpoint |
|---|---|
| Scheduled send (`send_at` on all 34 send endpoints) | `GET/DELETE /sessions/{sid}/scheduled`, `GET /scheduled` |
| Blast queue (bulk send, pacing, dedup, retry, DLQ) | `POST /sessions/{sid}/blast`, `GET /sessions/{sid}/blasts*`, `GET /blasts` |

### Voice & video calls

| Feature | Endpoint |
|---|---|
| Ring / hangup / accept / reject | `POST /sessions/{sid}/calls/*` |
| Text-to-speech call (Edge TTS, 300+ voices) | `POST /sessions/{sid}/calls/tts` |
| Audio playback call (MP3/WAV upload or URL) | `POST /sessions/{sid}/calls/play` |
| Native MLOW codec (WA proprietary, pure Rust) | internal |
| Peer audio recording (WAV) | `GET /sessions/{sid}/calls/{cid}/recording.wav` — local disk by default, S3-compatible object storage (AWS S3, MinIO, R2, …) when `S3_BUCKET` is set |
| Bidirectional media WebSocket stream (audio) | `WS /sessions/{sid}/calls/media/ws?to=&kind=audio` |
| Bidirectional media WebSocket stream (audio + video, H.264) | `WS /sessions/{sid}/calls/media/ws?to=&kind=av` — transport only, bring your own H.264 encoder/decoder (e.g. ffmpeg) on the client side |
| Transcript of a recording (external whisper.cpp server) | `POST /sessions/{sid}/calls/{cid}/transcript` — needs `WHISPER_API_URL`; waxum stays a pure-Rust single binary, no C++/whisper.cpp is compiled in |

### Session management

| Feature | Endpoint |
|---|---|
| Multi-session on one process | `POST /sessions` |
| Pair via QR (PNG + SVG) | `GET /sessions/{sid}/qr`, `/qr-svg` |
| Pair via 8-char code | `POST /sessions/{sid}/pair` |
| Auto-reconnect on start | env `WA_AUTO_RECONNECT_ON_START=1` |
| Fleet stats (uptime, per-status counts) | `GET /stats` |
| Search sessions (name/phone/JID) | `GET /sessions/search?q=` |
| Bulk purge (by status, older-than, dry-run) | `POST /sessions/purge` |
| Bulk disconnect / reconnect | `POST /sessions/{disconnect,reconnect}-all` |
| Re-enable all tripped webhook circuits | `POST /webhooks/reenable-all` |
| Export / import a session between instances (zip of local storage) | `POST /sessions/{sid}/export`, `POST /sessions/{sid}/import` |

### Groups

| Feature | Endpoint prefix |
|---|---|
| Create / leave / info / settings | `POST /sessions/{sid}/groups/*` |
| Member add / remove / promote / demote | `POST /sessions/{sid}/groups/{gid}/*` |
| Invite link create / revoke / join | `POST /sessions/{sid}/groups/{gid}/{invite,revoke,join}` |
| Description / subject / picture / announce / lock | `PATCH /sessions/{sid}/groups/{gid}/*` |
| Pending join requests approval | `POST /sessions/{sid}/groups/{gid}/requests` |

### Contacts / privacy / presence

| Feature | Endpoint |
|---|---|
| `is_on_whatsapp` batch check | `POST /sessions/{sid}/contacts/check` |
| Contact info + profile picture | `GET /sessions/{sid}/contacts/{jid}` |
| Sync device contacts | `POST /sessions/{sid}/contacts/sync` |
| Block / unblock / list blocked | `POST /sessions/{sid}/blocking/*` |
| Privacy settings (last-seen, profile, status) | `PATCH /sessions/{sid}/privacy` |
| Presence broadcast (online / offline) | `POST /sessions/{sid}/presence` |

### Media & storage

| Feature | Endpoint |
|---|---|
| Multipart upload (image/video/audio/document) | `POST /sessions/{sid}/media/upload` |
| Direct URL send (server downloads on your behalf) | `POST /sessions/{sid}/messages/image?url=` |
| Download inbound media by message id | `GET /sessions/{sid}/media/{mid}` |
| Media storage on disk (configurable path) | env `WA_MEDIA_DIR` |

### Webhooks

| Feature | Endpoint |
|---|---|
| Per-session webhook URL + secret | `POST /sessions/{sid}/webhooks` |
| HMAC-SHA256 signature (`X-Webhook-Signature`) | header |
| Event filter mask (message, receipt, call, presence, …) | `event_mask` field |
| Circuit breaker (auto-trip on Nx 5xx) | env `WEBHOOK_CB_THRESHOLD` |
| Dead-letter queue + replay | `GET /webhooks/dlq`, `POST /webhooks/dlq/replay` |
| Re-enable all tripped circuits (bulk) | `POST /webhooks/reenable-all` |

### Ops / observability

| Feature | Endpoint |
|---|---|
| Server-rendered ops console (Handlebars, no SPA) | `/` |
| Per-session playground covering 60+ endpoints | `/s/{sid}` |
| Swagger UI + OpenAPI 3.1 schema | `/swagger-ui` |
| Liveness / readiness probes | `/livez`, `/readyz` |
| Prometheus metrics (counters + gauges) | `/metrics` |
| Session tags (in-memory + JSON snapshot) | `GET/PUT/POST /api/v1/sessions/{sid}/tags`, `DELETE .../tags/{tag}`, `GET /api/v1/tags`, `GET /api/v1/sessions?tag=` |
| SSE event tail (filter by session / event) | `GET /api/v1/events/tail` |
| List Edge-TTS voices | `GET /api/v1/voices` |
| TTS voice preview (returns MP3) | `GET /api/v1/tts/preview?text=&voice=` |
| Instance-lock file (single-writer safety) | on-boot |
| FD-limit warning at start (nofile < 65536) | on-boot |
| JWT + static Bearer auth, per-token IP allowlist (planned) | header |
| NATS JetStream event fan-out (optional) | env `NATS_URL` |

### Storage backends

| Feature | Notes |
|---|---|
| SQLite (default, single-binary friendly) | `WA_DB=sqlite:///path.db` |
| Postgres (recommended for prod, > 50 sessions) | `WA_DB=postgres://…` |
| MySQL | `WA_DB=mysql://…` |

### Known limitations

| Gap | Why |
|---|---|
| Group voice call | **Blocked upstream** — `whatsapp-rust` has no multi-party relay/SFU client at all (single-peer engine only). Not a waxum-side gap; would need its own library-level project in `whatsapp-rust` first. |
| Encryption at rest | Not implemented. `libsqlite3-sys` (shared with `whatsapp-rust-sqlite-storage`) would need a `bundled-sqlcipher` build, which pulls in OpenSSL and breaks single-binary/cross-compile builds. Use OS/disk-level encryption (LUKS, BitLocker, encrypted volumes) instead — the standard approach for encryption at rest, and zero code changes. |

## Console

Server-rendered ops dashboard baked into the binary. Point a browser at
`http://<host>:3451/`, sign in with your `SUPERADMIN_TOKEN`, and you land
on the fleet overview. Click any session for the per-session playground
covering 60+ REST endpoints — send messages, drive calls, manage groups,
inspect webhooks, all without leaving the tab.

<p align="center">
  <img src="docs/screenshots/dashboard.png" alt="Waxum Console — fleet overview" width="820" />
</p>

<p align="center">
  <img src="docs/screenshots/playground.png" alt="Waxum Console — session playground" width="820" />
</p>

## Install

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/imtaqin/waxum/main/scripts/install.sh | sudo bash

# Windows (elevated PowerShell)
irm https://raw.githubusercontent.com/imtaqin/waxum/main/scripts/install.ps1 | iex

# Docker
docker pull fdciabdul/waxum
```

Or build from source:

```bash
git clone https://github.com/imtaqin/waxum && cd waxum
cargo build --release
./target/release/waxum
```

### Toolchain

**waxum builds on a pinned Rust nightly — `nightly-2026-04-05`.** You do
not have to select it: the pin lives in `rust-toolchain.toml`, and
`rustup` reads that file and installs the right toolchain on your first
`cargo build`. If you have `rustup`, there is nothing to do.

To install it up front, or to check what you are actually building with:

```sh
rustup toolchain install nightly-2026-04-05 --component rustfmt --component clippy
rustup show active-toolchain   # from the repo root; expect nightly-2026-04-05-<host>
```

Do not run `rustup default nightly`. That selects a floating latest
nightly globally, which is not what this project builds against.

**Without `rustup`, the pin does not apply.** `rust-toolchain.toml` is a
`rustup` feature; a distro-packaged, Homebrew, or Nix `cargo` ignores it
silently — no warning, no error, just a different compiler than the one
we test. If a build fails in a way this section does not explain, check
`cargo --version` before anything else.

If you force stable — `cargo +stable build` — we cannot tell you what
happens, because nobody has run it. Historically it failed inside
upstream `whatsapp-rust` with [`error[E0554]: #![feature] may not be used
on the stable release channel`](https://doc.rust-lang.org/error_codes/E0554.html),
which is the error this pin was introduced to prevent. That cause is gone
at the revision we now pin, so a stable build may well succeed — it is
simply unverified, and therefore unsupported.

The pin is historical. Upstream `whatsapp-rust` used the unstable
`portable_simd` feature, which is nightly-only; at the revision waxum
currently pins, upstream has removed SIMD from its tree and declares a
stable MSRV, and waxum itself uses no unstable features. The pin has not
been re-validated against stable, so it stays until someone does that
end to end. Tracked in
[#87](https://github.com/imtaqin/waxum/issues/87).

### Dependencies

The WhatsApp protocol layer is not ours. It is
[`whatsapp-rust`](https://github.com/oxidezap/whatsapp-rust), a
third-party project, and waxum pins eight of its crates to a single git
revision rather than to crates.io versions. That is a deliberate choice
with real consequences for anyone depending on waxum — including that
waxum cannot itself be published to crates.io. The reasoning, the risk,
and the bump cadence are written down in
**[docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)**.

## Endpoints

| URL | Purpose |
|---|---|
| `/` | Console — fleet overview + per-session playground |
| `/api/v1` | REST API |
| `/swagger-ui` | OpenAPI schema + interactive docs |
| `/livez` · `/readyz` | Liveness · readiness probes |
| `/metrics` | Prometheus text exposition |

## Stack

Rust nightly · Axum 0.8 · Tokio · [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) · Postgres/MySQL/SQLite · NATS JetStream · Prometheus · Utoipa.

## Ecosystem

| Repo | What it is |
|---|---|
| [waxum-studio](https://github.com/imtaqin/waxum-studio) | Visual WhatsApp workflow builder — nodes, integrations, drag-and-drop automation, powered by waxum. |
| [waxum-mcp](https://github.com/imtaqin/waxum-mcp) | MCP server for WhatsApp, backed by waxum — send/read/media tools for any MCP client (Claude Desktop, Claude Code, etc). |
| [waxum-sdk](https://github.com/imtaqin/waxum-sdk) | TypeScript SDK, types generated from waxum's OpenAPI spec. |
| [waxum-php-client](https://github.com/imtaqin/waxum-php-client) | PHP client for the waxum REST API. |
| [waxum-doc](https://github.com/imtaqin/waxum-doc) | Docs site — [waxum.imtaqin.id](https://waxum.imtaqin.id). |
| [waxum-hermes-plugin](https://github.com/imtaqin/waxum-hermes-plugin) | [Hermes Agent](https://github.com/NousResearch/hermes-agent) gateway platform plugin — real WhatsApp buttons/lists/CTA-url, which Hermes's built-in Baileys bridge can't do. |
| [waxum-openclaw-plugin](https://github.com/imtaqin/waxum-openclaw-plugin) | [OpenClaw](https://github.com/openclaw/openclaw) channel plugin — same interactive WhatsApp messaging, wired into OpenClaw's gateway. |

## Docs

Everything else — endpoints, webhooks, health probes, deployment,
`.env` reference — lives in the docs:

**[waxum.imtaqin.id](https://waxum.imtaqin.id)**

## License

MIT
