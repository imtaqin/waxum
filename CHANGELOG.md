# Changelog

All notable changes to **wa-rs** will be documented in this file.

## [0.6.3] - 2026-06-25

### New Features

#### SQLite default backend
- When `DATABASE_URL` (and the legacy `POSTGRES_*` / `MYSQL_*` env vars)
  are unset, wa-rs now defaults to an embedded SQLite file at `./wa-rs.db`
  instead of trying a Postgres `localhost` URL that almost never matches
  a clean checkout. Override the path with `SQLITE_PATH=/path/to.db` or
  set `DATABASE_URL=sqlite:///path/to.db`.
- `DbBackend::SQLite` joins `Postgres` and `MySQL`; every session,
  webhook, and contact query has a third match arm. Schema bootstraps
  itself on first connect (WAL journal, foreign keys on, the same
  three tables as the other backends).
- Implemented through a thin `src/db/sqlite_raw.rs` FFI wrapper over
  the `libsqlite3-sys 0.37` that `whatsapp-rust-sqlite-storage`
  already brings in — we can't pull `rusqlite` because it pins
  `libsqlite3-sys ^0.36`, and `links = "sqlite3"` allows only one copy
  of the native lib in the dep graph.

## [0.6.2] - 2026-06-25

### Upstream

- Bumped `whatsapp-rust` to oxidezap@302d478. Notable upstream changes
  picked up:
  - `feat(groups)`: backfill participant `phone_number` from LID-PN map
    so group participant listings carry the PN even when WhatsApp
    handed back only the LID.
  - `feat(retry)`: WA-Web parity log levels + retry-flow observability
    counters.
  - `refactor(error)!`: anyhow → per-domain typed errors in the public
    API surface.
  - `fix(atomics)`: portable_atomic for 64-bit atomics (matters on ARM
    and other non-x86 targets).
  - Multiple send-path perf wins (shared encode between reporting token
    and DM/SKMSG plaintext, skip DeviceSentMessage when there is no
    companion, group SKDM warm-gate one-lookup-per-device, signal
    cache flush without holding the device read-lock).
  - `Bot::run()` is now infallible and `Bot::with_backend(...)` takes
    `impl Backend` directly — `Arc::new(...)` wrappers are dropped at
    every call site.
  - Several inner protobuf message variants (Invoice, PaymentInvite,
    PinInChat, PollUpdate, ScheduledCallCreation/Edit,
    CancelPaymentRequest, DeclinePaymentRequest, Reaction) are now
    boxed (`Option<Box<T>>`) — every constructor call now wraps in
    `Box::new(...)`.

### New Features

#### Location data in webhook payload
- Inbound `location` and `live_location` messages now include a `location`
  object in the webhook event with `latitude`, `longitude`, plus optional
  `name`, `address`, `url`, `accuracy_meters`, `speed_mps`, and
  `is_live`. Live-location entries also carry `sequence_number` and the
  optional caption.

### Fixes / Reliability

#### Bounded webhook delivery
- Replaced the per-broadcast `reqwest::Client::new()` with a shared,
  `OnceLock`-initialised client carrying `timeout(10s)`,
  `connect_timeout(5s)`, and a small idle pool. The old setup let
  tokio tasks queue up against an unreachable webhook until the OS-level
  TCP timeout (~75 s), so a brief target outage piled threads (we
  observed ~600 on a quiet process). Each task is now bounded to ~10 s
  and the connection pool is reused across deliveries.

#### utf8mb4 on `contacts`
- Auto-`ALTER`s the text columns of the `contacts` table (`jid`,
  `phone`, `lid_jid`, `full_name`, `first_name`, `push_name`,
  `business_name`, `source`) to `utf8mb4_general_ci`. WhatsApp push
  names sometimes carry 4-byte characters (emoji etc); the previous
  `utf8mb3` setting tripped the upstream MySQL driver's collation
  conversion check and dropped the upsert. New tables are now created
  with `DEFAULT CHARSET=utf8mb4`.

## [0.6.1] - 2026-06-11

### New Features

#### Locally-cached contact list
- `GET /sessions/{id}/contacts` — paginated dump of the contacts wa-rs
  has seen for the session. Supports `?q=` (name / phone / business-name
  substring), `?limit=` (1-1000, default 100), `?offset=`.
- New `contacts` table backs the endpoint (PostgreSQL + MySQL). Schema:
  `(session_id, jid)` PK + `phone`, `lid_jid`, `full_name`,
  `first_name`, `push_name`, `business_name`, `source`, `updated_at`.
- Upserts happen automatically on:
  - `Event::ContactUpdate` (appstate sync mutation) — captures
    `full_name` / `first_name` / `lid_jid` from the saved address-book
    entry. `source` = `appstate_sync` on a full sync, `appstate`
    otherwise.
  - `Event::PushNameUpdate` — `push_name` refresh on each notification.
  - `Event::ContactUpdated` (`<notification type="contacts"><update/>`).
  - `Event::Message` — inbound only, captures `push_name` and the
    sender JID/phone so the directory fills organically alongside
    chats.

Filling the table is a side-effect of the existing event pipeline — no
extra `usync` round trips, no new background jobs.

### Rationale

Upstream `whatsapp-rust` only exposes per-JID lookups (`is_on_whatsapp`,
`get_user_info`) because WhatsApp itself doesn't return a contact list
over the socket; contacts come from the phone's address book via
appstate sync. The new endpoint wraps the appstate stream so callers
can hand out a directory without writing the persistence layer
themselves.

## [0.6.0] - 2026-06-11

### New Features

#### CAG channel reactions, both directions
- `/messages/reaction` now routes through `Client::send_reaction` which
  is CAG-aware: it auto-swaps to the encrypted addon stanza when the
  chat is a community-announce group and keeps the regular
  `ReactionMessage` path for 1:1 and standard groups.
- Inbound encrypted reactions on CAG channels are decrypted by the
  upstream lib and surfaced as `event=reaction` like every other
  reaction.

#### CAG channel comments
- `/messages/comment` rewritten to use `client.comments().send_text()`.
  Encrypted with the parent post's `messageSecret`, shipped as the
  top-level `enc_comment_message` envelope per WA Web parity.
- Each comment carries its own fresh `messageSecret` so it can itself
  receive reactions / replies.
- Request body gains an optional `target_participant` field for when
  the lib can't resolve the parent author from local msg_secret
  storage (e.g. commenting on someone else's first-seen post).
- Inbound encrypted comments are dispatched as their decrypted inner
  body via the normal `message` event with `comment_target` set on the
  envelope.

### Device props override on session create
- Already in 0.5.x, restated for the 0.6 release: pass `device` on
  `POST /sessions`, `/connect`, or `/pair` to override the OS /
  platform / app version shown in WhatsApp Linked Devices. Honored only
  on first pair.

### Upstream bump → oxidezap/whatsapp-rust@0aa6cb9
- Pulls in upstream send-path parity fixes (messageContextInfo hoisting
  in DeviceSentMessage, groupStatusV2 unwrap, payment-stanza
  classification, LID resolution clones cut, batched previous-MAC
  prefetch, libsignal verify-side memoization, group device-list
  topology memo, history-sync prost gating, ack-id borrowing, binary
  small-attr inline storage). Most are transparent perf wins.

### Internal / breaking call-site adjustments
- `Client::get_pn()`, `get_lid()`, `get_push_name()` are now sync — no
  longer `await`ed.
- `Client::download_from_params` takes a single `DownloadParams` struct
  built via `DownloadParams::encrypted(...)`.
- `Groups::query_info` returns `Arc<...>` — participants iterated by
  reference.
- `Groups::set_description` takes `Option<&str>` for `prev_id`.
- `Receipt::mark_as_read` takes `&[&str]` — handlers slice their owned
  Vec<String> into `&str`s.

## [0.4.7] - 2026-04-27

### New Features

#### Media metadata in webhook event payload
- [x] Inbound `image`, `video`, `audio`, `document`, `sticker` messages
      now include a `media` object in the webhook payload with
      `direct_path`, `media_key`, `file_sha256`, `file_enc_sha256`,
      `file_length`, `mimetype`, plus type-specific fields (width/height
      for image, seconds + ptt for audio, file_name for document).
- [x] All binary fields are base64-encoded so the webhook payload is
      transport-safe JSON.
- [x] Consumers can take this metadata directly to
      `POST /sessions/:id/media/download` to fetch the decrypted bytes
      without an extra metadata round trip.

### Rationale
Inbox UIs that want to render incoming images / play audio messages
need the encryption metadata; previously they had to call `/messages`
or build a chat dump pipeline. With media inline on the event,
end-to-end render is one round trip away.

## [0.4.6] - 2026-04-27

### New Features

#### Auto-reconnect on engine startup
- [x] On boot, the engine now walks every session in DB and spawns a
      background reconnect for any session that was previously logged-in
      or in a connecting state. Sessions that were never paired (or
      explicitly logged out) stay disconnected until manually paired.
- [x] Reconnects are staggered with a 500ms gap between sessions to
      avoid hammering WhatsApp's connection endpoint after a deploy.
- [x] Bumped to 0.4.6.

### Rationale
Engine restarts (deploys, crashes, host reboots) used to leave every
paired session offline until a human manually clicked "Connect" in the
dashboard for each one — painful with many sessions and lossy for users
expecting always-on operation.

## [0.4.5] - 2026-04-27

### Fixes

#### Webhook message events now carry actual content
- [x] `Event::Message` payload was previously serialized with only the
      envelope (`from`, `chat`, `message_id`, `timestamp`, `is_from_me`).
      The actual message body — `conversation`, `extended_text_message.text`,
      `image_message.caption`, etc. — was discarded entirely, so consumers
      had to call `/messages` to get content.
- [x] New helper `extract_message_content` reads the wa::Message protobuf
      and emits: `text`, `caption`, `message_type` (one of `text`, `image`,
      `video`, `audio`, `ptt`, `document`, `sticker`, `location`, `contact`,
      `contacts`, `poll`, `poll_vote`, `reaction`, `buttons`, `list`,
      `template`, `unknown`), and `media_mimetype`.
- [x] Event payload also adds `push_name`, `verified_name`, top-level
      `type`, `media_type`, `is_group`, and `participant`.

### Rationale
Inbox UIs that listened to webhooks couldn't render text/media without
re-fetching. With content inline, downstream consumers like MAUBLAST
can persist a real conversation log on a single round trip.

## [0.4.4] - 2026-04-26

### New Features

#### Custom device-props at pair time
- [x] New module `device_props` resolves how each session shows up in
      WhatsApp's "Linked Devices" list. Default is now `os = "Windows"`
      and `platform = Chrome` instead of the upstream `"rust"` /
      `Unknown` defaults.
- [x] Override globally via env: `WA_DEVICE_OS` (string, e.g. `Windows`,
      `Mac OS X`, `Ubuntu`) and `WA_DEVICE_PLATFORM` (one of `chrome`,
      `firefox`, `edge`, `safari`, `opera`, `ie`, `desktop`, `ipad`,
      `android_phone`, `android_tablet`, `ios_phone`).
- [x] Both QR-code and pair-code connect paths now call
      `Bot::builder().with_device_props(...)`. The pair-code
      `platform_display` string also picks up the resolved OS.

### Rationale
Previously sessions appeared as "Rust" / "Unknown device" in the
recipient's linked-devices list, which was both visually off-brand and
arguably more flagged-prone than mimicking a stock browser pairing.

## [0.4.3] - 2026-04-24

### New Features

#### Fake Reply Support for Media Messages
- [x] `SendImageRequest`, `SendVideoRequest`, `SendDocumentRequest` now accept
      an optional `fake_reply: FakeReplyConfig` field (same shape as
      `send_text`)
- [x] When `fake_reply` is set, the outgoing media message is wrapped with a
      synthesized `ContextInfo` so it appears as a reply to a fake product /
      order / location / video / document / contact / text message — same
      mechanic already used by `send_text`
- [x] `fake_reply` takes priority over `reply_to` when both are provided

### Rationale
Previously only `send_text` could produce the "fake reply" effect used by
blast to mimic natural conversation. Media sends (image/video/document) now
support the same trick.

## [0.4.2] - 2026-04-23

### New Features

#### Server Info Endpoint
- [x] `GET /api/v1/info` — public endpoint that returns server version and
      self-detected geo location (IP, country code/name, city, region, lat,
      lon, timezone)
- [x] Location auto-detection via ip-api.com on first request using the
      server's own outbound public IP — works correctly for Tailscale /
      overlay-network deployments where the inbound IP is private (100.64/10)
- [x] Env override support for all fields: `WA_LOCATION_CODE`,
      `WA_LOCATION_COUNTRY`, `WA_LOCATION_CITY`, `WA_LOCATION_REGION`,
      `WA_LOCATION_LAT`, `WA_LOCATION_LON`, `WA_LOCATION_TZ`, `WA_LOCATION_IP`
- [x] Result cached in process memory for the lifetime of the server

### Rationale
The adonis gateway previously did DNS resolve + ip-api lookup from its own
side to get server geo. That fails for servers behind Tailscale since
private IPs in the CGNAT range have no public geo info. Moving geo detection
into wa-rs itself means the detection uses the server's own outbound public
IP, which always works.

## [0.4.1] - 2026-04-23

### Fixes
- `send_message` handler: add missing `fake_reply: None` to `SendTextRequest`
  initializer (v0.4.0 introduced the field but one call site was missed,
  breaking compile)
- Clippy `let_and_return` lint in `random_stanza_id` — return expression
  directly instead of via `let` binding
- All `cargo fmt --check` issues flagged by CI (trailing array arg wrapping,
  method chain breaks, env var array formatting)

## [0.4.0] - 2026-04-22

### New Features

#### Fake Reply (Anti-Ban Message Wrapping)
- [x] `fake_reply` field on `POST /sessions/{id}/messages/text` wraps the
      outgoing text as a reply to a synthesized dummy message
- [x] 7 reply types: `text`, `product`, `order`, `location`, `video`,
      `document`, `contact`
- [x] Indonesian data pools bundled — 29 product names, 12 Jakarta
      malls/landmarks with real lat/long, 10 contact name templates,
      realistic IDR price ranges (Rp 15jt - 999jt)
- [x] Auto-generates participant JID and stanza ID when not provided
- [x] Takes precedence over `reply_to` when both present
- [x] New handler module `src/handlers/fake_reply.rs` with
      `build_fake_reply_context_info()` builder

#### Calls & Status Endpoints (scaffolding)
- [x] New `src/handlers/calls.rs` + `src/models/calls.rs` for call events
- [x] New `src/handlers/status.rs` + `src/models/status.rs` for status / story

#### Outbound HTTP Proxy
- [x] New `src/net.rs` module — reads `WA_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`
      env vars for outbound WA media / HTTP calls
- [x] CLI `--proxy <URL>` flag

### Internal
- Message `ContextInfo` now wrapped in `Box<>` for the recursive quoted_message type
- Cargo.toml adds `rand = "0.8"` for data pool selection
- Version bump 0.2.1 → 0.4.0 (aligns with CHANGELOG which had jumped to 0.3.0)

### Documentation
- Fake Reply section added to `docs/api/messages.md` with full type
  reference, field semantics, and cURL examples

## [0.3.0] - 2026-04-04

### Breaking Changes
- SQLite is no longer supported as metadata database (use PostgreSQL or MySQL)
- Replaced `sqlx` with native `tokio-postgres` + `mysql_async` drivers

### New Features

#### Multi-Database Support
- [x] PostgreSQL support via `tokio-postgres` + `deadpool-postgres`
- [x] MySQL support via `mysql_async`
- [x] `DATABASE_URL` env var — auto-detects backend from URL prefix
- [x] Legacy `POSTGRES_*` and `MYSQL_*` env vars as fallback
- [x] Auto-migration for MySQL column types on startup

#### CLI Arguments
- [x] `--token` / `-t` — set superadmin token from command line
- [x] `--db` / `-d` — set database URL from command line
- [x] `--port` / `-p` — set server port from command line
- [x] `--help` / `-h` — show usage help

#### Authentication
- [x] `SUPERADMIN_TOKEN` now works as plain string (no JWT required)
- [x] JWT validation still supported as fallback
- [x] Configurable server port via `PORT` env var

#### whatsapp-rust Upgrade (v0.2.0 → v0.5.0)
- [x] Newsletter/channel full CRUD and messaging support
- [x] Community support — full CRUD, subgroup management
- [x] Ephemeral message support (send and receive)
- [x] Sticker pack sending support
- [x] Album message support
- [x] Poll creation, voting, and vote decryption
- [x] Group invite V4 accept
- [x] Client logout API
- [x] Pin/unpin messages
- [x] Mark chat as read, delete chat, delete message for me
- [x] Presence unsubscribe and re-subscribe on reconnect
- [x] Media host failover and resumable upload
- [x] Better reconnection handling and keepalive
- [x] Signal protocol improvements and session caching
- [x] Performance optimizations (reduced allocations, SIMD support)
- [x] Stable Rust support for SIMD fallback
- [x] 150+ upstream bug fixes and improvements

#### Docker
- [x] Removed hardcoded credentials from Dockerfile and docker-compose
- [x] PostgreSQL service optional (separate compose file)
- [x] All config via `.env` file

#### CI/CD
- [x] Release workflow auto-creates git tags
- [x] Docker build optional (won't block release)
- [x] Auto-generated changelog in release notes
- [x] Proper release body rendering

---

## [0.2.0] - 2026-02-27

### New Features

#### Additional Message Types
- [x] Poll Update / Vote message — `POST /messages/poll-update`
- [x] Buttons response message — `POST /messages/buttons-response`
- [x] List response message — `POST /messages/list-response`
- [x] Interactive response message — `POST /messages/interactive-response`
- [x] Highly Structured Message (HSM) — `POST /messages/highly-structured`
- [x] Template button reply message — `POST /messages/template-button-reply`
- [x] Comment message (groups) — `POST /messages/comment`
- [x] Scheduled Call creation — `POST /messages/scheduled-call`
- [x] Scheduled Call edit/cancel — `POST /messages/scheduled-call-edit`

#### Business Payment Messages
- [x] Send Payment message — `POST /messages/send-payment`
- [x] Request Payment message — `POST /messages/request-payment`
- [x] Cancel Payment Request — `POST /messages/cancel-payment`
- [x] Decline Payment Request — `POST /messages/decline-payment`

#### Newsletter / Channel
- [x] Forwarded Newsletter message — `POST /messages/newsletter-forward`

#### Spam Reporting
- [x] Submit spam report — `POST /spam/report`

#### Trust Chain Token (TCToken) Management
- [x] Issue tokens — `POST /tctoken/issue`
- [x] Get token for JID — `GET /tctoken/{jid}`
- [x] List all JIDs with tokens — `GET /tctoken/list`
- [x] Prune expired tokens — `DELETE /tctoken/expired`

#### Reconnection Control
- [x] Get auto-reconnect status — `GET /reconnect`
- [x] Set auto-reconnect (enable/disable) — `PUT /reconnect`

#### History Sync Configuration
- [x] Get history sync setting — `GET /history-sync`
- [x] Set skip history sync — `PUT /history-sync`

#### Infrastructure
- [x] New OpenAPI tag: operations
- [x] All new endpoints documented in Swagger UI

---

## [0.2.0] - 2026-02-27

### New Features

#### Interactive & Rich Message Types
- [x] Poll messages (Create) — `POST /messages/poll`
- [x] Buttons message — `POST /messages/buttons`
- [x] List message (via Interactive NativeFlow) — `POST /messages/list`
- [x] Interactive message (generic NativeFlow) — `POST /messages/interactive`
- [x] Pin In Chat message — `POST /messages/pin`
- [x] Forward message — `POST /messages/forward`

#### Business Messages
- [x] Order message — `POST /messages/order`
- [x] Invoice message — `POST /messages/invoice`
- [x] Payment Invite message — `POST /messages/payment-invite`

#### Newsletter / Channel Support
- [x] Newsletter Admin Invite message — `POST /messages/newsletter-admin-invite`
- [x] Newsletter Follower Invite message — `POST /messages/newsletter-follower-invite`

#### Privacy Settings
- [x] Fetch privacy settings — `GET /privacy/settings`

#### GraphQL / MEX Operations
- [x] Execute GraphQL queries — `POST /mex/query`
- [x] Execute GraphQL mutations — `POST /mex/mutate`

#### Advanced Group Settings
- [x] Group settings endpoint — `PUT /groups/{group_jid}/settings`
- [x] Membership approval mode (On / Off) in create_group
- [x] Member add mode (AdminAdd / AllMemberAdd) in create_group
- [x] Member link mode (AdminLink / AllMemberLink) in create_group

#### Additional Webhook Events
- [x] PictureUpdate
- [x] UserAboutUpdate
- [x] PushNameUpdate / SelfPushNameUpdated
- [x] ContactUpdate
- [x] DeviceListUpdate
- [x] PinUpdate
- [x] MuteUpdate
- [x] ArchiveUpdate
- [x] MarkChatAsReadUpdate
- [x] UndecryptableMessage
- [x] ClientOutdated
- [x] OfflineSyncPreview
- [x] OfflineSyncCompleted

#### Infrastructure
- [x] Updated Rust nightly toolchain to 2026-01-30 (matches whatsapp-rust)
- [x] OpenAPI tags: privacy, mex, newsletter
- [x] All new endpoints documented in Swagger UI
- [x] All group management endpoints registered in OpenAPI

---

## [0.1.1] - 2026-02-27

### Implemented Features

#### Session Management
- [x] Multi-session support (manage multiple WhatsApp accounts)
- [x] Session creation with custom ID and friendly name
- [x] Session listing with status information
- [x] Session status (real-time connection state)
- [x] Session deletion with storage cleanup
- [x] Session persistence (creation timestamp, last connection time)

#### Authentication
- [x] QR Code authentication (device linking via scan)
- [x] Pair Code authentication (8-character code with phone number)
- [x] Push notification option during pairing
- [x] Connection state tracking (Disconnected, Connecting, WaitingForQr, WaitingForPairCode, Connected, LoggedIn)
- [x] Bearer token (JWT) authentication for API
- [x] Superadmin token (env or auto-generated)

#### Messaging
- [x] Text messages
- [x] Image messages (with caption)
- [x] Video messages (with caption)
- [x] Audio messages (with voice note/PTT support)
- [x] Document messages (with filename and caption)
- [x] Sticker messages (WebP format)
- [x] Location messages (lat/long with optional name and address)
- [x] Contact messages (vCard format)
- [x] Reply to specific messages (`reply_to`)
- [x] Edit sent messages
- [x] Revoke/delete messages (sender or admin)
- [x] Message reactions (emoji)
- [x] Mark messages as read

#### Media Handling
- [x] Media upload (multipart form data)
- [x] Media download (encrypted retrieval and decryption)
- [x] Automatic media type detection (Image, Video, Audio, Document, Sticker)
- [x] URL-based media sending
- [x] Base64 encoded media sending
- [x] Pre-uploaded media with encryption keys
- [x] SHA-256 file hashing
- [x] MIME type handling

#### Group Management
- [x] List participating groups
- [x] Get group info / metadata
- [x] Create groups with initial participants
- [x] Set group subject
- [x] Set group description
- [x] Leave group
- [x] Add participants (with status tracking)
- [x] Remove participants
- [x] Promote to admin
- [x] Demote from admin
- [x] Get / reset invite link

#### Contact Management
- [x] Check if phone numbers are on WhatsApp
- [x] Get contact info (JID, LID, registration, business status, picture ID)
- [x] Get profile picture
- [x] Get user info (status, business account)
- [x] Batch contact operations
- [x] Business account detection

#### Presence & Chat State
- [x] Set presence (Available / Unavailable)
- [x] Subscribe to presence updates
- [x] Last seen tracking
- [x] Typing indicator (Composing, Recording, Paused)
- [x] Per-chat state indicators

#### Blocking
- [x] Block contacts
- [x] Unblock contacts
- [x] Get blocklist
- [x] Check block status

#### Webhook System
- [x] Register webhooks per session
- [x] List active webhooks
- [x] Unregister webhooks
- [x] Enable/disable webhooks
- [x] HMAC-SHA256 signature verification
- [x] Event filtering per webhook
- [x] Webhook events: Message, Receipt, Presence, ChatPresence, GroupUpdate, GroupJoin, QrCode, PairCode, Connected, Disconnected, LoggedOut

#### Dashboard (Web UI)
- [x] Session management interface
- [x] QR code display and scanning
- [x] Pair code display
- [x] Device info display
- [x] Session status badges
- [x] Login authentication
- [x] Settings page
- [x] Session statistics (Total, Connected, Disconnected)
- [x] Quick actions (Connect, Disconnect, Delete, Pair)

#### API & Infrastructure
- [x] OpenAPI 3.0 / Swagger UI (`/swagger-ui`)
- [x] Health check endpoint (`/health`)
- [x] CORS support
- [x] Request tracing / logging
- [x] PostgreSQL for session & webhook storage
- [x] SQLite per-session for WhatsApp state
- [x] Docker support (Dockerfile + docker-compose)
- [x] Environment variable configuration (`.env`)

---

## TODO - Remaining features

### Message Types (not yet exposed via REST)
- [ ] Bot messages (BotDocument, BotFeedback, BotPromotion)
- [ ] AI Rich Response message
- [ ] Secret Encrypted message
- [ ] Marketing message

### New in whatsapp-rust v0.5.0 (not yet exposed)
- [ ] Newsletter/channel CRUD REST endpoints
- [ ] Community CRUD REST endpoints
- [ ] Privacy settings update (now type-safe enums)
- [ ] Client logout endpoint
- [ ] Mark chat as read / delete chat endpoints

### Architecture Improvements
- [ ] Pluggable transport layer
- [ ] Pluggable HTTP client
