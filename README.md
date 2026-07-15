
<p align="center">
  <img src="https://waxum.imtaqin.id/img/logo.png" alt="Waxum" width="160" />
</p>

<h1 align="center">Waxum</h1>

<p align="center">
  <strong>High-performance multi-session WhatsApp Gateway built with Rust</strong><br>
  <em>Because life's too short for garbage collection.</em>
</p>

<p align="center">
  <a href="https://waxum.imtaqin.id/">Documentation</a> &bull;
  <a href="https://waxum.imtaqin.id/api/nats">NATS JetStream</a>
</p>

## Tech Stack

| Component | Technology |
|-----------|------------|
| Runtime | Rust (Nightly) |
| Web Framework | Axum 0.8 |
| Database | PostgreSQL 14+ / SQLite |
| Message Queue | NATS JetStream (optional) |
| API Docs | OpenAPI 3.0 / Swagger UI |
| WhatsApp | whatsapp-rust (unofficial) |
| Auth | JWT Bearer Token |

## Features

- **Multi-session** — Manage multiple WhatsApp accounts simultaneously
- **QR Code & Pair Code** — Two authentication methods for linking devices
- **Rich Messages** — Text, images, video, audio, documents, stickers, location, contacts, polls, legacy buttons, lists, native-flow interactive (CTA URL, quick reply), payments, and more (30+ message types)
- **Webhooks** — Real-time events with HMAC-SHA256 signature verification
- **NATS JetStream** — Optional durable event streaming and queue-based outbound messaging
- **Swagger UI** — Interactive API documentation at `/swagger-ui`
- **Group Management** — Create groups, manage participants, admins, and settings
- **Privacy & Blocking** — Privacy settings, block/unblock contacts
- **Advanced Ops** — Spam reporting, TCToken, auto-reconnect, history sync, GraphQL/MEX

## Premium (Pro / Enterprise)

Everything above is MIT-free forever. On top of it waxum offers a paid
tier that activates through a signed license key — no source access
required, drop the key into the environment and the runtime unlocks the
extra features.

| Capability | Free | Pro | Enterprise |
|---|:-:|:-:|:-:|
| Multi-session gateway, all message types, webhooks, NATS | ✓ | ✓ | ✓ |
| Prometheus `/metrics`, DLQ, circuit breaker | ✓ | ✓ | ✓ |
| **Anti-ban shield** (adaptive throttle, typing simulation, burst cool-off) | — | ✓ | ✓ |
| **Webhook DLQ replay UI + admin API** | — | ✓ | ✓ |
| **Encrypted backup** (AES-256-GCM tar.zst → S3 or local) | — | ✓ | ✓ |
| **AI auto-reply** (OpenAI-compatible: OpenAI, Kimi, Claude via proxy, Ollama) | — | — | ✓ |
| **Multi-node cluster** (session sharding across N waxum instances) | — | — | ✓ |
| Priority support (private Slack, response SLA) | — | community | ✓ |

**How to activate**

1. Grab a license key at [waxum.imtaqin.id/pricing](https://waxum.imtaqin.id/pricing) — currently invite-only, DM to onboard.
2. Set two env vars and restart:
   ```
   WA_RS_LICENSE_KEY=<your license>
   # WA_RS_LICENSE_PUBKEY=<optional issuer pubkey to enforce signed keys>
   ```
3. Startup log confirms activation:
   ```
   premium: tier=pro features=[anti_ban,dlq_replay,backup] (signed)
   ```

Everything premium is compiled into the binary released on this repo —
the source stays closed under a commercial licence. Free-tier binaries
run identically, they just log `premium: free tier` at boot.

## Quick Start

### One-liner install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/imtaqin/waxum/main/scripts/install.sh | sudo bash
```

Fetches the latest GitHub Release binary, drops it in `/usr/local/bin`,
writes a systemd unit at `/etc/systemd/system/waxum.service`, generates
`/etc/waxum.env` with a random JWT + superadmin token, and offers to
enable a nightly auto-update cron. Same script handles `update` and
`uninstall`:

```bash
sudo /usr/local/bin/waxum-update       # pull latest release + restart
sudo bash install.sh uninstall         # remove service + binary (data kept)
```

### One-liner install (Windows, elevated PowerShell)

```powershell
irm https://raw.githubusercontent.com/imtaqin/waxum/main/scripts/install.ps1 | iex
```

Downloads the latest release, installs into
`C:\ProgramData\waxum`, registers a Windows service (`waxum`) that
auto-starts on boot, and — if you say yes — installs a scheduled task
for nightly auto-update at 03:15. Subcommands:

```powershell
.\install.ps1 update      # check + apply latest release
.\install.ps1 uninstall   # remove service (data kept)
```

Both installers ship the same interactive banner and remember whether
you opted into auto-update, so the update path is a single `Enter` on
subsequent runs.

### Docker Compose

Pulls the prebuilt image from Docker Hub — no compile step, comes up in seconds.

```bash
git clone https://github.com/imtaqin/waxum.git
cd waxum
docker compose up -d
```

Pin a version with `WA_RS_TAG=0.5.0 docker compose up -d`. Default is `latest`.

This starts **NATS JetStream** and the **Waxum API** server (bring your own MySQL/Postgres in `.env`).

### Docker Compose (build from source)

When iterating on the Rust code, layer the build override on top — compiles the local checkout instead of pulling the image:

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up -d --build
```

### Manual

```bash
git clone https://github.com/imtaqin/waxum.git
cd waxum
cp .env.example .env    # edit with your config
cargo build --release
./target/release/waxum
```

**Requirements:** Rust nightly, PostgreSQL 14+

## Access Points

| URL | Description |
|-----|-------------|
| `http://localhost:3451/api/v1` | REST API |
| `http://localhost:3451/swagger-ui` | Swagger UI |
| `http://localhost:3451/health` | Health Check |
| `http://localhost:3451/api/v1/nats/status` | NATS Status |

## Environment Variables

### Core

| Variable | Default | Description |
|----------|---------|-------------|
| `POSTGRES_HOST` | `localhost` | PostgreSQL host |
| `POSTGRES_PORT` | `5432` | PostgreSQL port |
| `POSTGRES_USER` | `postgres` | PostgreSQL user |
| `POSTGRES_PASSWORD` | `postgres` | PostgreSQL password |
| `POSTGRES_DB` | `wagateway` | Database name |
| `JWT_SECRET` | *(random)* | JWT signing secret |
| `SUPERADMIN_TOKEN` | *(random)* | Fixed superadmin token |
| `WHATSAPP_STORAGE_PATH` | `./whatsapp_sessions` | Session storage path |
| `RUST_LOG` | `info` | Log level |

### NATS JetStream (Optional)

Omit `NATS_URL` to disable NATS entirely — the API runs in webhooks-only mode.

| Variable | Default | Description |
|----------|---------|-------------|
| `NATS_URL` | *(none)* | NATS server URL |
| `NATS_EVENTS_STREAM` | `WA_EVENTS` | Incoming events stream |
| `NATS_SEND_STREAM` | `WA_SEND` | Outbound commands stream |
| `NATS_EVENTS_MAX_AGE_DAYS` | `7` | Events retention (days) |
| `NATS_SEND_MAX_AGE_DAYS` | `1` | Outbound retention (days) |
| `NATS_TOKEN` | *(none)* | Auth token |
| `NATS_CREDS_FILE` | *(none)* | Credentials file |

## NATS JetStream

Waxum optionally integrates with NATS JetStream for durable event streaming and queue-based messaging.

```
Your App  ◄──── wa.events.{session}.{type} ────  Waxum  ◄──── WhatsApp
Your App  ────► wa.send.{session}           ────► Waxum  ────► WhatsApp
```

**Subscribe to events:**
```bash
nats sub "wa.events.>"
```

**Send a message via NATS:**
```bash
nats pub "wa.send.my-session" '{"type":"text","to":"628123456789","text":"Hello from NATS!"}'
```

See the [NATS documentation](https://waxum.imtaqin.id/api/nats) for all 16 supported message types, consumer details, and send result format.

## API Overview

| Category | Endpoints |
|----------|-----------|
| Sessions | Create, list, connect, disconnect, QR code, pair code, device info |
| Messages | Text, image, video, audio, document, sticker, location, contact, poll, legacy buttons, list, native-flow interactive, CTA URL button, quick reply buttons, reaction, edit, revoke, read, pin, forward, payment, scheduled call, newsletter, and more |
| Contacts | Check on WhatsApp, get info, profile picture, user info |
| Groups | Create, list, info, participants, admins, settings, invite link |
| Presence | Set online status, subscribe to presence |
| Chat State | Typing and recording indicators |
| Blocking | Block/unblock contacts, blocklist |
| Privacy | Get privacy settings |
| Media | Upload and download encrypted media |
| Webhooks | Register, list, delete with HMAC-SHA256 signing |
| MEX | GraphQL query and mutate |
| Operations | Spam report, TCToken, auto-reconnect, history sync |
| NATS | Status, stream purge, consumer listing |

## Documentation

Full documentation: **[https://waxum.imtaqin.id/](https://waxum.imtaqin.id/)**

## License

MIT
