
<pre align="center">
██╗    ██╗ █████╗       ██████╗ ███████╗
██║    ██║██╔══██╗      ██╔══██╗██╔════╝
██║ █╗ ██║███████║█████╗██████╔╝███████╗
██║███╗██║██╔══██║╚════╝██╔══██╗╚════██║
╚███╔███╔╝██║  ██║      ██║  ██║███████║
 ╚══╝╚══╝ ╚═╝  ╚═╝      ╚═╝  ╚═╝╚══════╝
</pre>

<p align="center">
  <strong>High-performance multi-session WhatsApp Gateway built with Rust</strong><br>
  <em>Because life's too short for garbage collection.</em>
</p>

<p align="center">
  <a href="https://wa-rs.imtaqin.id/">Documentation</a> &bull;
  <a href="https://wa-rs.imtaqin.id/api/nats">NATS JetStream</a>
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

## Quick Start

### Docker Compose (Recommended)

```bash
git clone https://github.com/fdciabdul/wa-rs.git
cd wa-rs
docker compose up -d
```

This starts **PostgreSQL**, **NATS JetStream**, and the **WA-RS API** server.

### Manual

```bash
git clone https://github.com/fdciabdul/wa-rs.git
cd wa-rs
cp .env.example .env    # edit with your config
cargo build --release
./target/release/wa-rs
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

WA-RS optionally integrates with NATS JetStream for durable event streaming and queue-based messaging.

```
Your App  ◄──── wa.events.{session}.{type} ────  WA-RS  ◄──── WhatsApp
Your App  ────► wa.send.{session}           ────► WA-RS  ────► WhatsApp
```

**Subscribe to events:**
```bash
nats sub "wa.events.>"
```

**Send a message via NATS:**
```bash
nats pub "wa.send.my-session" '{"type":"text","to":"628123456789","text":"Hello from NATS!"}'
```

See the [NATS documentation](https://wa-rs.imtaqin.id/api/nats) for all 16 supported message types, consumer details, and send result format.

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

Full documentation: **[https://wa-rs.imtaqin.id/](https://wa-rs.imtaqin.id/)**

## License

MIT
