# Changelog

All notable changes to **wa-rs** will be documented in this file.

## [0.3.0] - 2026-02-27

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

## TODO - Remaining features from whatsapp-rust

### Message Types (not yet exposed via REST)
- [ ] Bot messages (BotDocument, BotFeedback, BotPromotion) — FutureProofMessage wrapper, complex metadata
- [ ] AI Rich Response message — complex sub-message types (grid images, code, tables, maps, LaTeX)
- [ ] Sticker Pack message — requires media upload/encryption for pack data
- [ ] Secret Encrypted message — requires encryption key handling
- [ ] Marketing message — sync action type, not a direct sendable message
- [ ] Poll Results snapshot — read-only aggregated poll results

### Privacy Settings
- [ ] Set/update individual privacy settings — not available in whatsapp-rust library (read-only)

### Sync Features (handled internally by whatsapp-rust)
- [ ] App state sync REST endpoints — whatsapp-rust handles internally, could expose state queries
- [ ] Dirty bits tracking REST endpoints — `clean_dirty_bits()` available but mostly internal

### Architecture Improvements
- [ ] Pluggable storage backends (currently hardcoded SQLite + PostgreSQL)
- [ ] Pluggable transport layer
- [ ] Pluggable HTTP client
- [ ] Device consistency / code verification — not available in whatsapp-rust library
