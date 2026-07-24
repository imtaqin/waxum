# Changelog

All notable changes to **waxum** will be documented in this file.

## [0.9.2] - 2026-07-24

### Added — S3 backend for call recordings

- Call recordings can now live in S3-compatible object storage (AWS
  S3, MinIO, R2, Wasabi, …) instead of local disk. Set `S3_BUCKET`
  (plus `S3_ENDPOINT`, `S3_REGION`, `AWS_ACCESS_KEY_ID`,
  `AWS_SECRET_ACCESS_KEY`) to switch; omit it and recordings keep
  writing to `WHATSAPP_STORAGE_PATH` as before. A connection failure
  at startup logs an error and falls back to local disk rather than
  aborting the process, matching how NATS is wired up.
- New `src/storage.rs`, pulled in via the `s3` crate — pure Rust,
  rustls by default, no native TLS/OpenSSL and no C build step (kept
  in mind after the whisper-rs incident in 0.9.0/0.9.1).
- Scope note: this only covers recordings. Message media (images,
  video, documents) was never persisted to local disk in the first
  place — it streams straight through to WhatsApp's own CDN via
  `client.upload`/`download_from_params` — so there was nothing else
  under "media" to back with S3.

## [0.9.1] - 2026-07-24

`v0.9.0`'s tag exists but was never actually released — its build
pulled in `whisper-rs`, which compiles whisper.cpp (C++) into the
binary and broke the Windows cross-compile job in CI. Superseded by
this version before any binaries or Docker images went out.

### Added — video calling + call transcription

- **Video calls** — `calls/media/ws?kind=av` places a call with both
  audio and video, relaying raw H.264 Annex-B access units to/from the
  peer over the existing media WebSocket. Waxum is transport-only here
  (matching the upstream `whatsapp-rust` design): the WS client brings
  its own H.264 encoder/decoder (e.g. ffmpeg). Frames are tagged with a
  1-byte media type so audio and video share one socket; `kind=audio`
  keeps the original untagged raw-PCM wire format unchanged.
- **`POST /sessions/{sid}/calls/{cid}/transcript`** — forwards a call
  recording to an external whisper.cpp-compatible HTTP server
  (`WHISPER_API_URL`) and relays back its `text`. Keeps waxum a
  pure-Rust single binary with no C++ toolchain or model file baked
  in — run whisper.cpp (or any compatible server) as its own service,
  same shape as the existing Edge-TTS integration.
- Group voice calls stay off the roadmap: `whatsapp-rust` has no
  multi-party relay/SFU support at the library level at all, so this
  isn't implementable as a waxum-side feature without a much larger
  upstream project first.

### Fixed

- `tokio-stream` was missing the `sync` feature it silently depended
  on via `ReceiverStream`.
- `tts_preview` returned 500 for an unknown voice name; now 400, since
  that's a client input error, not a server fault.
- `list_voices` re-queried Edge-TTS on every request; the voice list
  is now cached after first fetch and the response carries a
  `cache-control` header.
- SQLite FTS5 search silently fell back to the snippet-less `LIKE`
  path because the join SELECT used unqualified column names
  (`ambiguous column name` at prepare time). Columns are now qualified
  with `m.`.

### Changed

- `whatsapp-rust` pin bumped from `4d9e8ed` to `0077186` (22 upstream
  commits): buffa 0.9 + `SyncdOperation`, extensible plugin client
  architecture, typed USync query engine, signal record components +
  dirty events, typed legacy session interop, targeted retransmission
  controls, business template/buttons/list/interactive message
  classification, PN/LID alias resolution for receipts, several
  allocation/binary-size perf passes.

## [0.8.0] - 2026-07-21

### Added — scheduled send

Every send endpoint now accepts an optional `send_at` ISO-8601 UTC
timestamp. When it lies more than a couple of seconds in the future
the request is parked in a new `scheduled_messages` table
(tri-backend: Postgres / MySQL / SQLite) instead of sending, and a
background scheduler dispatches it once due — through the same
`execute_*` send core the immediate path uses, so a parked `text`
and an immediate `text` are identical on the wire.

- **`send_at` field on all 34 send endpoints** — `text`, `image`,
  `cta-url`, interactive, payments, and everything in between. A
  `send_at` inside the 2-second grace window (or in the past) still
  sends immediately.
- **Unified `SendResponse`** — immediate sends answer
  `status: "sent"` with `message_id` / `timestamp` / `to` as before;
  parked sends answer `status: "pending"` with `schedule_id` and the
  effective `send_at`.
- **Scheduler worker** — `tokio::time::interval` loop with a poll
  period from `SCHEDULER_POLL_MS` (default 1000 ms). Claims due rows
  atomically (`pending` → `sending`) so a concurrent cancel or a
  second poller loses the race, then settles each row `sent` or
  `failed`.
- **`GET /api/v1/sessions/{sid}/scheduled`** — list parked messages
  for one session, optional `?status=` filter.
- **`DELETE /api/v1/sessions/{sid}/scheduled/{id}`** — cancel one
  that is still `pending`; 400 once the scheduler has claimed it.
- **`GET /api/v1/scheduled`** — fleet-wide list with `?session=` and
  `?status=` filters.
- **`scheduled_sent` / `scheduled_failed` webhook events** — fired on
  dispatch either way, carrying the schedule id, endpoint, and the
  message id or the error.

### Added — blast queue engine

Bulk-send one message payload to many recipients with pacing, dedup,
retry, and a dead-letter queue. A blast job stores the endpoint key
plus the original JSON body once, and one row per recipient in
`blast_recipients`; a single sequential worker (deliberate —
WhatsApp rate-limit and ban safety) drains it in batches of 50,
replaying each recipient through the same dispatch the scheduler
uses, with `to` rewritten per recipient and any `send_at` inside the
body forced off.

- **`POST /api/v1/sessions/{sid}/blast`** — create a job. Body
  `{endpoint, body, recipients, delay_ms?, jitter_ms?, max_attempts?, dedup_across_jobs?, send_at?}`.
  `endpoint` is any scheduler dispatch key (`text`, `image`,
  `cta-url`, …) — unknown keys and bodies that fail the endpoint's
  shape validation are a 400. Recipients are JID-validated (the 400
  lists the bad ones) and deduped within the array; with
  `dedup_across_jobs: true`, recipients any previous blast of the
  same session already delivered to are skipped too. An optional
  `send_at` on the job delays the start.
- **Pacing** — `delay_ms` base pause between sends (default 1000)
  plus a uniform random `0..=jitter_ms` extra per send.
- **Retry + DLQ** — a failed send goes back to `pending` until
  `max_attempts` (default 3) is exhausted, then lands in `dlq` with
  the last error; only the retry endpoint requeues it.
- **`GET /api/v1/sessions/{sid}/blasts`** — list jobs for one
  session, optional `?status=` filter.
- **`GET /api/v1/sessions/{sid}/blasts/{id}`** — job detail with
  live counters (total, sent, failed, dlq, skipped_dup).
- **`GET /api/v1/sessions/{sid}/blasts/{id}/recipients?status=&limit=&offset=`** —
  paginated recipient list.
- **`POST /api/v1/sessions/{sid}/blasts/{id}/cancel`** — stop a
  pending or running job; unprocessed recipients stay `pending` and
  never send.
- **`POST /api/v1/sessions/{sid}/blasts/{id}/retry`** — requeue all
  `dlq` / `failed` recipients with attempts reset and reopen the
  job.
- **`GET /api/v1/blasts`** — fleet-wide job list with `?session=`
  and `?status=` filters.
- **`blast_progress` / `blast_completed` webhook events** — progress
  with counters every 25 sends, and a final summary when the job
  closes (`completed` or `completed_with_failures`).
- **`BLAST_POLL_MS`** — worker poll interval in milliseconds
  (default 1000).

### Added — message history + full-text search

Inbound and outbound messages are now persisted to a `messages` table
(tri-backend Postgres / MySQL / SQLite) with a body-indexed FTS
projection. SQLite uses native `FTS5`; Postgres uses `tsvector` +
`GIN`; MySQL uses `FULLTEXT`. Snippet highlighting and rank ordering
are returned in the same shape across all three.

- **`GET /api/v1/sessions/{sid}/messages/search?q=&limit=&before=`** —
  per-session search, newest-first, with highlighted snippet and
  rank score.
- **`GET /api/v1/messages/search?q=&session=&limit=`** — fleet-wide
  search across every session.
- **Schema** — new `messages` table (id, session_id, chat_jid,
  sender_jid, direction, body, mime, timestamp) with matching FTS
  index per backend; migrations run automatically on boot.

### Changed — TypeScript SDK split out

The auto-generated TypeScript SDK now lives in its own repository,
[imtaqin/waxum-sdk](https://github.com/imtaqin/waxum-sdk). `sdk/`
and `scripts/gen-sdk.sh` are no longer part of this tree; the
OpenAPI schema at `/api-docs/openapi.json` remains the source the
SDK generator consumes.

## [0.7.13] - 2026-07-20

### Added — session tags

Operators managing tens or hundreds of sessions have had no way to
group them for filtering. This release adds a lightweight tag
system: any short label (`cs`, `blast-campaign-2`, `client:acme`,
`region:jkt`) can be attached to a session, and the session listing
gains a `?tag=` filter.

Tags are held in-memory on `AppState` and snapshotted to
`{WHATSAPP_STORAGE_PATH}/session_tags.json` on every mutation, so a
restart preserves organisation. A future release will promote them
to first-class DB rows once the surface stabilises; for now the
JSON file is authoritative on disk and read once at startup.

- **`GET /api/v1/sessions/{sid}/tags`** — list the tags on a
  session. Returns `{session_id, tags: [...]}` with tags sorted
  alphabetically.
- **`PUT /api/v1/sessions/{sid}/tags`** — replace the full tag
  set. Body `{"tags": ["cs", "team:jakarta"]}`. Empty strings and
  whitespace-only entries are dropped; sending an empty array
  removes the session from the tag map entirely.
- **`POST /api/v1/sessions/{sid}/tags`** — add one tag. Body
  `{"tag": "vip"}`. Response reports `changed: true|false` so the
  caller can distinguish a real add from an already-present tag.
- **`DELETE /api/v1/sessions/{sid}/tags/{tag}`** — remove one
  tag. Session drops out of the map once its last tag is gone.
- **`GET /api/v1/tags`** — enumerate every distinct tag across
  the fleet with the number of sessions carrying it, sorted by
  count then name. Powers a "tag cloud" in the console.
- **`GET /api/v1/sessions?tag=<t>`** — the existing session
  listing accepts `?tag=<name>` and returns only sessions carrying
  that tag. Case-sensitive, trimmed on both sides.

Deleting a session (`DELETE /api/v1/sessions/{sid}`) now also
drops any tags associated with it, so the JSON snapshot does not
accumulate orphans.

## [0.7.12] - 2026-07-19

### Added — event tail + TTS discovery

Two operator-focused endpoints and a new discovery surface for the
Edge-TTS pipeline that already backs `/calls/tts`.

- **`GET /api/v1/events/tail`** — server-sent event stream over the
  in-memory event ring. Every event that reaches
  `broadcast_to_webhooks` is emitted as an SSE line
  (`event:` = event type, `id:` = epoch ms, `data:` = JSON preview).
  Supports `?session=<sid>` and `?event=<name>` filters so an
  operator can `curl -N /api/v1/events/tail?session=foo` and watch
  one session in isolation. Sends the last 50 items as backlog on
  connect so late subscribers see a tail immediately; keep-alive
  every 15 s to survive intermediary idle-timeouts. Complements
  webhooks — no delivery guarantees, but no config either.
- **`GET /api/v1/voices`** — list every voice Edge-TTS exposes
  (~600 entries). Fields returned: `name`, `short_name` (the value
  callers pass to `/calls/tts`), `locale`, `gender`,
  `friendly_name`. Cache aggressively client-side; the list is
  stable per Edge-TTS release.
- **`GET /api/v1/tts/preview?text=<t>&voice=<v>`** — audition a
  voice without placing a call. Returns the raw MP3 that Edge-TTS
  produced (`Content-Type: audio/mpeg`), so an operator can hit the
  endpoint straight from the console `<audio>` tag and hear the
  voice before wiring it to a live call. `text` is capped at 500
  chars — this is a preview, not the send path.

### Notes

Console updates that surface these endpoints in the playground land
alongside the v0.7.12 binary; nothing existing is behind a feature
flag or breaking.

## [0.7.11] - 2026-07-19

### Added — fleet-level endpoints

Every REST endpoint before this release was scoped to one session
via `/api/v1/sessions/{session_id}/...`. Cross-session ops (bulk
purge, disconnect-all, search-by-phone, aggregate stats) had no
place. This release adds a `fleet` tab that operates over the whole
gateway at once.

- **`POST /api/v1/sessions/purge?filter=<inactive|logged_out|disconnected|all>&days=<N>&dry_run=<bool>`** —
  Bulk delete sessions matching the filter. `inactive` (default) is
  "not logged in AND last activity older than `days`". `dry_run=true`
  returns the list without deleting so an operator can review the
  target set first. Storage directories and metadata rows go
  together, matching the single-session `DELETE /sessions/{sid}`
  semantics.
- **`POST /api/v1/sessions/disconnect-all`** — Force disconnect
  every session that currently holds a live Client. Used for
  maintenance windows before a restart. Sessions with no client
  are reported under `skipped`.
- **`POST /api/v1/sessions/reconnect-all`** — Trigger the same
  `reconnect_all_on_startup` routine that runs on boot, without
  restarting the process. Every session with `is_logged_in = true`
  gets an auto-reconnect spawn (staggered by
  `SESSION_STARTUP_STAGGER_MS`).
- **`GET /api/v1/sessions/search?q=<needle>`** — Substring +
  case-insensitive match against session id, name, phone number,
  and push name. Each hit reports which field(s) matched under
  `match_on`, so operators can tell why a session came back.
- **`GET /api/v1/stats`** — Fleet aggregate: total / connected /
  connecting / disconnected / logged-out session counts,
  webhook total, open circuits, event rate in the last minute,
  uptime seconds, waxum version, storage path.
- **`POST /api/v1/webhooks/reenable-all`** — Close every open
  webhook circuit at once. Used after fixing a mass downstream
  outage that tripped many circuits.

Playground picks up a new **Fleet** tab with all six endpoints;
purge and disconnect-all are marked as `danger` in the form so a
double-click is required.

## [0.7.10] - 2026-07-19

### Added — voice calls actually work now

- **Native MLOW audio codec on `/calls/tts` and `/calls/play`.**
  Waxum encodes PCM straight to WhatsApp's proprietary MLOW frames
  via `wacore::voip::MlowEncoder` (pure Rust, 960 samples / 60 ms)
  and uses `AudioFormat::MLOW_16KHZ_60MS` on the outbound call.
  Earlier iterations were silent on the callee side: the raw
  `.audio()` PCM path never carried audio, and the Opus MLOW-escape
  variant reached the peer with zero packet loss yet still refused
  to play (peer's decoder wanted native MLOW, not Opus/CELT). This
  is codec parity with meowcaller.
- **Peer audio recording.** `record: true` on `/calls/tts` and
  `/calls/play` spawns an MLOW-decoder task that captures the
  peer's inbound audio and writes it as a 16 kHz mono WAV file at
  `{WHATSAPP_STORAGE_PATH}/{session_id}/recordings/{call_id}.wav`.
  The file is served over
  `GET /api/v1/sessions/{sid}/calls/{cid}/recording.wav`. Empty-audio
  placeholder is always written so the download endpoint returns
  `200` right after the call ends even when the peer never spoke.
- **Bidirectional media WebSocket.**
  `GET /api/v1/sessions/{sid}/calls/media/ws?to=<phone_or_jid>`
  upgrades to a WebSocket that carries raw PCM in both directions:
  client sends 16 kHz mono `s16le`, server pushes peer audio in the
  same shape as it arrives. First frame back is a JSON metadata
  blob (sample rate, frame samples, encoding). Ideal for
  full-duplex voice agents (LLM + STT + TTS loops) driving live WA
  calls.

### Added — CTA URL header + optional body

- **`POST /messages/cta-url` now accepts a `header_text` field** so
  the header line above the body can carry a different string than
  the button label. When omitted it falls back to `display_text` for
  backward compatibility with existing callers.
- **`body_text` is now optional.** Setting it to `""` (or leaving it
  out) produces a header + button-only interactive block — useful
  when the image + header + CTA are all the message needs to say.

### Added — browser console pair widget

- **Dedicated `/s/{sid}` Pair panel.** Above the tab bar, visible
  when the session is not logged in, showing an auto-refreshing
  QR SVG (fetched from a new `GET /qr-svg/{sid}` console route that
  renders the runtime QR through the same qrcode helper the drawer
  uses) and a phone-number → pair-code flow. Pair code displays as
  hyphenated `XXXX-XXXX` with a live 180-second countdown; grays
  out on expiry.
- **`pair_with_code` reuses the running client.** The `/pair`
  handler used to spawn a fresh Bot on every request, which raced
  the previous bot's pair state and returned an invalid code half
  the time. It now calls `Client::pair_with_code` directly on the
  cached client when one exists, spawns a Bot only for cold
  sessions, and polls the runtime pair-code cache for up to 20 s
  before returning. Response `timeout_seconds` corrected from 60 to
  180 to match upstream.

### Added — playground quality-of-life

- **Live audio player in the response pane** — when a call
  response includes a `recording_url`, the playground appends an
  HTML5 `<audio controls>` element that fetches the WAV directly.
  No curl round-trip to hear the peer.
- **Pair form** — phone number + "Show push notification" toggle,
  writes correctly to `POST /pair` (the old form sent `phone` but
  the server DTO wanted `phone_number`).
- **Trim + JSON-escape on inputs** — single-line text/select
  fields are trimmed before submit, JSON fields validated. Trailing
  whitespace no longer causes a 500 in the send handlers.

### Changed — TTS pipeline hardening

- **SSML text is now XML-escaped** before being passed to
  `msedge-tts`. The upstream crate string-formats the caller's
  text straight into the SSML body; any `<`, `>`, `&`, `"`, `'` in
  the payload silently returned 0-byte audio from Microsoft. Waxum
  now maps them to entities and strips U+FEFF / control chars.
- **3× retry with 500 ms backoff** on synth failure or empty
  response. Reconnect is fresh WebSocket each attempt. Recovery on
  attempt ≥ 2 is logged at `INFO waxum::tts`.
- **`ffmpeg` gets `-f mp3` explicitly** so Windows builds don't
  crash the format autoprobe (the exit-code `0xbebbb1b7` you saw
  in the field). Its stderr is now captured and surfaced in the
  API error message.

### Changed — call recipient LID resolution

- **New `resolve_call_recipient` helper** used by every call
  handler. Walks `is_on_whatsapp` first (which also persists the
  PN↔LID mapping into `lid_pn_cache` for future calls to skip the
  round-trip), then `get_user_info` as fallback. Both are bounded
  by explicit timeouts (8 s / 4 s) so a bad JID or a slow WA
  server doesn't hang the request. Fixes the "no known LID for the
  PN callee; cannot derive media keys" error on `/calls/tts`,
  `/calls/play`, `/calls/ring`, and `/calls/media/ws`.

### Changed — session status semantics

- **Auto-reconnect is now on by default** on every freshly built
  Client. The `Client::enable_auto_reconnect` flag was previously
  opt-in via `PUT /reconnect`; new sessions and every restart now
  set it to `true` at build time. Whoever wants the old opt-in
  behaviour can flip it off with the same endpoint.
- **Transient socket drops no longer show up as "OFFLINE".** A
  `LoggedIn` session whose WebSocket has died surfaces as
  **`Connecting`** across `GET /sessions`, `GET /sessions/{id}`,
  `GET /sessions/{id}/status`, and the console header — not
  `Disconnected`. The account is still paired at WhatsApp's end
  and whatsapp-rust is rebuilding the socket in the background;
  the UI should say so instead of scaring the operator. Only an
  explicit `LoggedOut` event (three flaps → purge) leaves the
  cache in `Disconnected` and stops the polling loop from
  auto-repairing the state.
- **`Event::Disconnected` handler no longer drops the cached Client
  Arc** or writes `is_logged_in = false` to the metadata DB. That
  earlier eager-drop was what turned every 3-second network blip
  into a red pill on the console and a "session gone" row in the
  operator's tail. The Client Arc lives so whatsapp-rust can
  reopen the WebSocket in place; the DB row keeps the previous
  `is_logged_in` value until a real `LoggedOut` arrives.
- **`Event::Disconnected` now logs the underlying
  `DisconnectReason`.** Clean recycles (`StreamEnded`,
  `ServerClose { code: 1000 | 1001 }`) are `INFO`; anything else
  stays `WARN` so a real transport failure isn't hidden by the
  reconnect noise.

### Changed — observability

- **`ApiError::into_response` emits a structured tracing event**
  before axum wraps it into a 5xx/4xx. Internal / SessionError /
  MediaUploadFailed / MediaDownloadFailed / NatsError fire
  `tracing::error!(target: "waxum::api_error", ...)`;
  BadRequest / InvalidJid fire `tracing::warn!`. The `tower_http`
  on-failure line still shows the status but the actual cause is
  now on the line above it.
- **Startup stagger default lowered 500 ms → 100 ms.** A 2-3
  session fleet no longer waits 1.5 s of nothing between spawns.
  `SESSION_STARTUP_STAGGER_MS` is still honoured for large fleets
  that want more headroom against WA rate limits.

### Fixed

- **Session status drift between `GET /sessions`, `GET /sessions/{id}`,
  and the console header vs. `GET /sessions/{id}/status`.** The first
  three read the cached `SessionStatus` enum straight from
  `SessionState`, which drifted whenever the socket dropped
  silently — the runtime kept `LoggedIn` on file even though the
  live client said otherwise. Only `/status` did the right thing.
  A new `SessionState::effective_status()` helper does the
  reconciliation once and is now used by all four call sites.

## [0.7.8] - 2026-07-17

### Added — browser console

- **Server-rendered ops console mounted at `/`.** A single-binary
  Handlebars UI over the existing REST surface, no separate frontend
  build. Landing page shows a fleet overview (speech-bubble hero with
  the connected/total count, active-webhook + open-circuit KPIs, a
  live-events tail panel, and a sessions table). Auth is a
  `waxum_console` cookie carrying `SUPERADMIN_TOKEN`; the same JWT
  middleware now accepts that cookie as a fallback to
  `Authorization: Bearer`, so browser fetches from the console UI
  hit `/api/v1/...` without exposing the token to JavaScript.
- **Per-session playground at `/s/{session_id}`.** Nine tabs (Info &
  Pair, Send, Chat, Contacts, Groups, Calls, Blocking, Webhooks,
  Operations) covering 60+ REST endpoints. Every endpoint is
  described once in `src/console/assets/playground.js`; forms are
  rendered dynamically from that registry, and responses stream
  back into a JSON panel.
- **Manga-panel visual system.** Hand-drawn ink borders on a mint
  paper background, jade accent for connected/OK, sakura for
  attention states, halftone dot texture on the speech-bubble hero.
  Logo taken from `logoini.png` (light-theme variant). Zen Maru
  Gothic + Inter + JetBrains Mono type pairing.

### Added — media plane

- **CTA URL messages now accept an image header.**
  `POST /api/v1/sessions/{sid}/messages/cta-url` accepts an optional
  `image` field (`{ "url": "…" }` or
  `{ "data": "<base64>", "mimetype": "image/jpeg" }`). When set,
  waxum uploads the image to the WhatsApp CDN and attaches it as
  the interactive header media, so the button appears with a
  thumbnail on the recipient side.
- **`/calls/tts` no longer requires an external `edge-tts` CLI or
  Python interpreter.** Speech synthesis now runs through the
  in-process `msedge-tts` Rust crate, which talks the Microsoft
  Edge readaloud WebSocket directly. Server prerequisites drop to
  just `ffmpeg` (for MP3 → PCM decode). Voice IDs are validated
  against the live catalogue; a bad voice returns a clear error
  listing valid alternatives instead of the previous opaque
  "program not found".

### Observability

- **Every WhatsApp event is now logged to the terminal, not only
  forwarded to webhooks.** `broadcast_to_webhooks` pushes each event
  into a bounded ring buffer (last 200) and emits a
  `tracing::info!` at `target = "waxum::event"` with the session id,
  event type, and a 160-char payload preview. The console overview
  "Live events" panel is backed by this same ring, so operators see
  live activity without needing to register a webhook at all.

## [0.7.7] - 2026-07-17

### Fixed — flap causes

Three classic reasons a waxum instance kept dropping WhatsApp connections
in the field were left to bite operators one at a time. This release
adds startup diagnostics + a hard interlock for the ones we can prevent.

- **Two instances on the same Signal Store no longer coexist.** waxum
  now takes a pidfile at `{WHATSAPP_STORAGE_PATH}/.waxum.lock` on
  startup. If another *live* process already owns that path, the new
  instance refuses to boot with a clear error. Previously the two
  processes would fight — WhatsApp servers issue `<failure
  reason='replaced'/>` against whichever session was last to connect,
  so the two peers would knock each other offline in a permanent
  loop, indistinguishable from a "random" disconnect storm.
  Stale pidfiles (owner PID no longer running) are silently reclaimed
  so a hard-killed container that never cleaned up on exit does not
  brick the next start.
- **`RLIMIT_NOFILE` is checked on Linux startup.** A soft limit under
  65536 logs a `WARN` with the estimated session ceiling (~soft / 70)
  and the exact docker-compose `ulimits:` block to add. A 1024-fd
  container wedges at ~14 sessions with brand-new connections
  silently failing to open, which is by far the most-diagnosed
  cause of "sessions disconnect but WA app never shows a logout".
- **Cold-start reconnect burst is tunable.** The per-session spawn
  stagger inside `reconnect_all_on_startup` used to be hard-coded to
  500 ms. It is now driven by `SESSION_STARTUP_STAGGER_MS` (default
  still 500 ms). Ops teams running >500 sessions on shared infra can
  raise this to 2000 ms so the WA rate limiter never sees the full
  fleet reconnect at once after a restart.

New knob: `SESSION_STARTUP_STAGGER_MS` (integer, ms).

## [0.7.6] - 2026-07-17

### Fixed

- **Sessions vanishing after container restart** (reported in issue
  #34 as a follow-up). The default SQLite metadata file used to live
  next to the binary (`./waxum.db`), which meant a Docker Compose
  setup that only mounted `whatsapp_sessions/` kept the Signal Store
  but lost the session metadata table on restart. The default is now
  `{WHATSAPP_STORAGE_PATH}/waxum.db` (default
  `./whatsapp_sessions/waxum.db`), so a single volume mount covers
  both buckets. Parent directory is auto-created on boot. `DATABASE_URL`,
  `SQLITE_PATH`, and the Postgres/MySQL env pairs still override.

## [0.7.5] - 2026-07-17

### Upstream sync

- Bumped pinned `whatsapp-rust` from `a7dc852` to `4d9e8ed`
  (13 commits ahead). New in upstream: **1:1 video calls** (#1024) and
  the encoded-audio pipeline with native Opus negotiation (#1050).
  Ten more perf/fix commits in the signal + message hot paths, plus a
  CI-side disk reclamation before tests. No API surface change on the
  waxum side.

### Tests

- Added an integration test harness under `tests/`. Every endpoint that
  does not require a live WhatsApp client now has a contract test
  running against an in-process `AppState` backed by a per-test SQLite
  file. Twenty-four tests cover the auth gate, `/livez` + `/readyz` +
  `/health` + `/metrics`, `/api/v1/info`, sessions CRUD, session status
  shape parity, and webhooks CRUD + re-enable 404.

### CI

- `Run tests` step now runs before `Build (release)` so a failing test
  fails the build fast instead of after ten minutes of release LTO.
- `Clippy` now runs `--all-targets` so the integration tests get linted
  the same way as the binary.

## [0.7.4] - 2026-07-16

### CI

- Release binary lane now runs the ARM64 job on `ubuntu-24.04-arm` (native
  runner) instead of `cross` under QEMU emulation. Dropped `Cross.toml`
  and the `cross` bootstrap step. ~5–8× faster on the ARM64 job.
- Docker publish is now a per-arch parallel matrix (`linux/amd64` on
  `ubuntu-latest`, `linux/arm64` on `ubuntu-24.04-arm`) followed by a
  `docker-manifest` job that stitches the two into a single
  `imtaqin/waxum:{version}` and `:latest` manifest via
  `docker buildx imagetools create`. End-to-end Docker publish went from
  ~90 min (single QEMU job) to ~20 min (parallel native jobs). Reported
  in #31.

## [0.7.3] - 2026-07-16

### Fix

- `POST /messages/cta-url` now delivers to the recipient. The outbound
  `NativeFlowMessage` was missing `message_params_json = "{\"tag\":
  \"cta_url\"}"` and the parent `InteractiveMessage` was missing its
  `header` (`title` + `has_media_attachment=false`). WA silently dropped
  the CTA button without them — the send would return a message id but
  nothing ever landed on the receiver. Reported in #31.

## [0.7.2] - 2026-07-15

### CI

- Docker image now publishes to `fdciabdul/waxum` on Docker Hub.

## [0.7.1] - 2026-07-15

### CI

- Added `cmake` + `build-essential` to the Linux release lane, set
  `CMAKE_POLICY_VERSION_MINIMUM=3.5` globally, and added a
  `Cross.toml` that installs cmake inside the ARM64 cross container.
  This unblocks the audiopus / opus build script on every runner.
- Dockerfile installs cmake in the rust-builder stage and exports the
  same policy env so `docker build` succeeds too.

## [0.7.0] - 2026-07-15

### Waxum

- **Rebrand.** The project is now `waxum`. Package name, binary name,
  banner, docs domain (`waxum.imtaqin.id`), GitHub org
  (`imtaqin/waxum`), metric prefix (`waxum_*`), README, Docker image,
  everything renamed. `wa-rs` references only survive as historical
  changelog entries.

### Upstream sync

- Bumped pinned `whatsapp-rust` revision from `f95eb2d` to `a7dc852`
  (12 commits ahead — mostly hot-path perf: message allocations,
  signal fast paths, receipt worker, node ack sizing, per-branch send
  dispatch, binary marshal cache, group SKDM warmth, coalesced signal
  flushes, batch outbound counter leases). Public API compatible; no
  code changes required in waxum. New: `add_lid_pn_mapping` is now
  public for embedder-learned sources, and upstream ships a
  SQLite-backed chat/message history store that we do not consume yet.

## [0.6.19] - 2026-07-10

### Fix TTS / play "menghubungkan ulang" loop

- The 4-second grace period between the `<offer>` ack and the first PCM
  chunk was a hard silence gap that made WhatsApp think the media path
  never came up — the callee saw "menghubungkan ulang" (reconnecting)
  forever even after answering.
- Rework the pipeline: generate/decode the audio BEFORE sending the
  offer, prefix `answer_grace_ms` worth of silent PCM, then push a
  continuous stream from the moment the media relay is up. The channel
  never goes empty, WhatsApp treats it as an active call, and the peer
  hears silence → real audio in one contiguous stream.
- Default `answer_grace_ms` bumped from 4000 to 6000 to accommodate
  average pickup latency.

## [0.6.18] - 2026-07-10

### Play arbitrary audio on a call

- New endpoint `POST /calls/play` — ring a peer and, once the media
  relay is up, play back an audio file fetched from `audio_url` (any
  format ffmpeg can demux: mp3, wav, ogg, m4a, opus…). Terminates the
  call after the last PCM chunk drains.
- Complements `/calls/tts` for anything that's not text-to-speech —
  pre-recorded voice messages, alerts, jingles, IVR clips.

## [0.6.17] - 2026-07-10

### TTS voice calls

- New endpoint `POST /calls/tts` — rings a peer and speaks a piece of
  text at them using Microsoft Edge Neural voices. Request body:
  `{ "to": "6285...", "text": "...", "voice": "id-ID-ArdiNeural",
  "answer_grace_ms": 4000 }`. Default voice is Indonesian
  (`id-ID-ArdiNeural`); see `edge-tts --list-voices` for the full
  catalog.

  Pipeline: `edge-tts` writes an MP3 to stdout, `ffmpeg` transcodes it
  to raw 16-bit signed PCM at 16 kHz mono, and the handler pushes 20 ms
  chunks (320 samples) into the VoIP mic channel. After the last chunk
  is drained, the call hangs up. The `answer_grace_ms` param gives the
  callee a few seconds to accept before playback starts.

  Requires `edge-tts` and `ffmpeg` on `PATH` at runtime.

## [0.6.16] - 2026-07-10

### Calls now use the upstream VoIP facade

- `POST /calls/ring` now calls `client.voip().call(peer).audio(...).start()`
  from the upstream `whatsapp-rust` VoIP module instead of hand-rolling
  the raw `<call><offer>` signalling stanza. The old stanza was missing
  the encrypted callKey per peer device, the privacy token, the net
  hint, the capability blob, and the device-identity signature — WA's
  server silently dropped it, so the peer phone never rang.

  The new path generates a callKey, wraps it in a Signal `<enc>` per
  peer device, and emits the full offer WA Web sends. Peer phone rings
  for real. Audio is fed by empty async channels: the offer succeeds
  but no PCM ever flows, so the callee sees ring → silent connect →
  timeout, or connects briefly if they answer.

- `POST /calls/reject` and `POST /calls/accept` now look up the
  matching `IncomingCall` in a new in-memory registry (indexed by
  `call_id`) and call `voip().reject(&incoming)` /
  `voip().accept(&incoming).audio(...).start()`. The registry is
  populated by the `Event::IncomingCall` handler so consumers always
  have the `MediaOffer` material the VoIP path needs to decrypt the
  peer's callKey.

- `POST /calls/terminate` looks up the live `CallHandle` in a second
  in-memory registry (populated by ring / accept) and calls
  `.hangup().await`; falls back to `voip().terminate(...)` when the
  handle isn't tracked (e.g. after a restart).

### Build

- Added the `voip` feature to the pinned `whatsapp-rust` dep. Pulls
  in `webrtc-sctp`, `webrtc-dtls`, and `audiopus_sys` (opus codec),
  so the debug binary is ~5 MB heavier and the Windows dev build now
  needs the MSVC toolchain + CMake with
  `CMAKE_POLICY_VERSION_MINIMUM=3.5` for the audiopus FFI.

## [0.6.15] - 2026-07-09

### Calls

- Video kind on `POST /sessions/{id}/calls/ring`. Send
  `{"to": "...", "kind": "video"}` to make the peer's phone ring
  with the video-call incoming UI. Adds a `<video enc="vp8" ...>`
  codec child to the offer. Default remains `"audio"`.

### CI

- Explicit `rustup target add x86_64-pc-windows-gnu` in the Windows
  release job. The `dtolnay/rust-toolchain@nightly` `targets:` input
  did not actually fetch the GNU std libs on the `windows-latest`
  runner, so the release lane failed with
  `can't find crate for core / std`.

## [0.6.14] - 2026-07-09

### Upstream sync

- Bumped the pinned `whatsapp-rust` revision from `302d478` to `f95eb2d`
  (112 commits ahead). Notable upstream changes now available:
  - 1:1 voice calling with real WebRTC media, not just signaling.
  - Protocol surface bump to `2.3000.1042742319`.
  - LID-PN mapping learned from history-sync `phoneNumberToLidMappings`,
    with source-aware write policy matching WA Web.
  - `ServerAck` observe-only event exposing server-side `<ack>` stanzas.
  - Fix: sender-key map no longer memoizes the account's own devices,
    matching WA Web parity.
- The event payload API is now sealed with `#[non_exhaustive]` + `bon`
  builders across the notification, sync, and ServerAck payloads. All
  event consumers in this crate were ported to the new shape.
- The message proto now uses `protobuf::MessageField<T>` in place of
  `Option<Box<T>>`. All handlers (`handlers::messages`,
  `handlers::fake_reply`, `handlers::status`, `handlers::operations`)
  and the NATS producer were migrated. No REST API surface change.

## [0.6.13] - 2026-07-09

### Windows build

- Switched Windows release target from `x86_64-pc-windows-msvc` to
  `x86_64-pc-windows-gnu`. The MSVC binary depends on
  `VCRUNTIME140.dll` which many Windows machines don't ship with,
  producing a "DLL not found" error at first launch. The GNU target
  links against the MinGW runtime statically, so the release exe now
  starts on a clean Windows without any redistributable install.

## [0.6.12] - 2026-07-09

### Webhook subsystem hardening

- **Auto-disable dead webhook targets.** After 100 consecutive delivery
  failures for the same URL the circuit escalates from OPEN (5 min
  cooldown) to a hard disable: the DB row switches to `enabled=false`,
  fresh `disabled_at` + `disabled_reason` columns record the reason, and
  the in-memory dispatcher purges every registration pointing at that
  URL. Stops the "orphan 127.0.0.1:3452 keeps getting hammered for
  months" behaviour. Manual recovery via
  `POST /api/v1/sessions/{sid}/webhooks/{wid}/enable`.
- **Session delete now cascades.** `DELETE /sessions/{id}` explicitly
  purges child rows (`webhooks`, `contacts`, `webhook_dlq`) before
  removing the `sessions` row and also drops the in-memory webhook
  registry for that session. Belt-and-suspenders on top of the FK
  cascade, so tables migrated in without the CASCADE clause still get
  cleaned up.

### Probes

- **`/livez` (liveness) + `/readyz` (readiness) split.** `/health` still
  answers `"OK"` for backward compat, but new deploys should use
  `/livez` (pure static probe, no dependencies) as the liveness gate
  and `/readyz` (runs a `SELECT 1` against the DB pool + reports the
  session-runtime count in JSON) as the readiness gate.

## [0.6.11] - 2026-07-04

### Fixes

- **`DATABASE_URL=sqlite://…` was routed to the Postgres client.** The
  detector only recognised `mysql://` and fell through to Postgres for
  everything else, so a fresh install with the default SQLite path
  exited with `invalid connection string / unexpected EOF`. SQLite and
  `file:` prefixes now map to the SQLite backend.

## [0.6.10] - 2026-07-04

### New

- **`POST /calls/accept`** and **`POST /calls/terminate`** — write the
  `<call><accept/>` and `<call><terminate reason="…"/>` signalling
  stanzas via `Client::send_node`. Still signalling only (no RTP media
  stack), so accept then terminate is the useful pair — the call
  becomes "answered" in the recipient's log without any audio flowing.
- **`GET /webhooks` returns IDs.** Payload switched from
  `Vec<WebhookConfig>` to `Vec<WebhookConfigWithId>` so clients can
  drive `DELETE /webhooks/{id}` off the response directly — no more
  round-tripping through the local cache to look the id up. New
  `WebhookConfigWithId` schema is registered on OpenAPI.

## [0.6.9] - 2026-07-04

### New

- **`POST /calls/ring`** — signalling-only outbound ring. Builds the
  `<call><offer>` stanza (opus 16k + 8k audio codecs, generated call-id)
  and pushes it through `Client::send_node`. The recipient's phone
  rings until WhatsApp times out, since the upstream `whatsapp-rust`
  crate has no RTP media stack yet. Useful for number verification and
  attention pings. Returns the `call_id` so the caller can later
  `/calls/reject` if needed.

## [0.6.8] - 2026-06-30

### Resilience

- **Webhook circuit breaker.** Each target URL now tracks consecutive
  failures across all sessions. After 25 consecutive failed deliveries
  the URL's circuit opens for 5 minutes — incoming events skip dispatch
  entirely (still recorded to DLQ) until the cooldown expires. A single
  successful response resets the counter. Stops the gateway from
  saturating its tokio task queue when a webhook receiver is dead
  (RENTALWA :8862 incident bled hundreds of retry tasks per minute).
- **/metrics gains `waxum_webhook_circuits_open`** — scrape this to alert
  when a destination has been declared dead.

## [0.6.7] - 2026-06-29

### Scale

- **Tokio runtime tuning.** Replaced `#[tokio::main]` with a manual
  `Builder::new_multi_thread()`, exposing `WA_RS_WORKER_THREADS` (default
  CPU count) and `WA_RS_BLOCKING_THREADS` (default 2048, up from tokio's
  512). 200+ active WhatsApp sessions doing SQLite + MySQL writes
  saturated the default blocking pool — now we have 4× headroom.
- **Prometheus `/metrics`.** New endpoint exposes
  `waxum_sessions_total`, `waxum_sessions_live`,
  `waxum_process_threads`, `waxum_process_open_fds`. Bypasses the JWT
  middleware so a scraper can hit it without a token.

### Fixes

- **Duplicate `connect_client` spawn.** `/connect` previously only
  short-circuited when `is_alive()` was true. When the runtime was mid-
  bootstrap (`Connecting` / `WaitingForQr` / `WaitingForPairCode`), a
  second `/connect` call spawned a second bot — leading to the
  `Connected → QR → Connected → QR` flap seen on `user_585_c786d4d6`.
  Now any in-progress state also returns 409.

## [0.6.6] - 2026-06-29

### Operability

- **MySQL pool tuning.** Default mysql_async pool capped at 10 conns and
  never rotated; under burst it starved sessions on remote MySQL
  (Jakarta ↔ AWS-SG, ~30–60 ms RTT). Pool now constrained 4–64 (override
  via `WA_RS_MYSQL_MAX_POOL`) with 5 min inactive TTL and a 60 s TTL
  check sweep, so stale idle conns rotate before MySQL's wait_timeout
  kills them silently.
- **LoggedOut auto-purge backoff.** v0.6.4 wiped storage on the first
  `LoggedOut` event, which spun infinite QR-rescan loops when WhatsApp
  flapped a session briefly. Purge now requires 3 logouts within 10 min;
  a single transient flap keeps the storage row and lets the user retry.
- **Pair flow telemetry.** `/status` now returns a `pair` object with
  `last_qr_at`, `last_pair_code_at`, `pair_code_expires_at`, `attempts`,
  and `last_error` so backends can render meaningful pair progress
  instead of polling `/qr` blindly.
- **Webhook DLQ.** Failed webhooks that exhaust retry now land in a
  `webhook_dlq` table (Postgres / MySQL / SQLite) with payload,
  last_error, and attempt count, so operators can replay them.

## [0.6.5] - 2026-06-26

### Fixes

#### State desync between /status and send handlers (P0)
- `/status` and `/messages/text` could disagree about the same session:
  status would say `logged_in: true` while the next send returned 503
  "Client not connected" — the symptom backend operators saw as
  "Terkirim padahal pesan gak nyampe". Root cause: the cached `Arc<Client>`
  was never cleared on `Event::Disconnected`, so the live-connection
  check on the send path returned `Some` of a dead handle, while the
  status flag separately stayed `LoggedIn`.
- Introduced `SessionState::is_alive()` and `get_live_client()` that
  consult the upstream `Client::is_connected()` + `Client::is_logged_in()`
  flags as a single source of truth.
- Every send / contact / chatstate / media / presence / group / op
  handler now resolves its client through `get_live_client()`. /status
  reports `logged_in` only when the live client agrees.
- `Event::Disconnected` now also calls `set_client(None)` so the cached
  handle and the socket state stop disagreeing within the same process.
- `/connect` 409 only fires for sessions that are actually alive — a
  dead cache lets the caller re-bootstrap instead of locking them out.

#### Docker HEALTHCHECK (P0)
- Dockerfile now ships a HEALTHCHECK probing `/health` every 30 s. The
  `/health` handler is a static-string endpoint that bypasses the DB
  pool, so docker can recycle a container whose tokio executor has
  stalled even when the PID is still alive. Covers the "31 hours
  online, every request times out" failure mode.

#### Webhook delivery retry (P1)
- Webhook dispatch now retries with exponential backoff (0 s, 1 s, 3 s,
  7 s, max 4 attempts). 4xx responses other than 408/429 short-circuit
  to avoid burning retries on permanent wedge bugs in the receiver.

## [0.6.4] - 2026-06-26

### Fixes

#### Auto-purge logged-out sessions
- When the upstream `LoggedOut` event fires, the session now self-destructs:
  the live client is disconnected, the in-memory runtime is dropped, the
  on-disk session directory is removed, and the database row is deleted.
  Previously the session lingered as `Disconnected` and every reconnect
  attempt kept trying to re-bind a dead device — which in production
  manifested as repeating `connection failed: database connection error`
  loops on `user_499_b4e9046e` / `user_9_f6e7fd99` and friends, eventually
  hanging the gateway. Now the user just rescans a fresh QR.

## [0.6.3] - 2026-06-25

### New Features

#### SQLite default backend
- When `DATABASE_URL` (and the legacy `POSTGRES_*` / `MYSQL_*` env vars)
  are unset, waxum now defaults to an embedded SQLite file at `./waxum.db`
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
- `GET /sessions/{id}/contacts` — paginated dump of the contacts waxum
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
into waxum itself means the detection uses the server's own outbound public
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
