# Changelog

All notable changes to **wa-rs** will be documented in this file.

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
