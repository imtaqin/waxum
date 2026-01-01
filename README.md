<p align="center">
  <img src="assets/logo.jpg" alt="WA-RS Logo" width="200">
</p>

<h1 align="center">WA-RS</h1>

<p align="center">
  High-performance multi-session WhatsApp Gateway built with Rust, why?
 because life's too short for garbage collection.
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#authentication">Authentication</a> •
  <a href="#api-usage">API Usage</a> •
  <a href="#dashboard">Dashboard</a>
</p>

## Tech Stack

| Component | Technology |
|-----------|------------|
| Runtime | Rust (Nightly) |
| Web Framework | Axum 0.8 |
| Database | PostgreSQL 14+ |
| Template Engine | Askama |
| API Docs | OpenAPI 3.0 / Swagger UI |
| WhatsApp | whatsapp-rs (unofficial) |
| Auth | Bearer Token |

## Features

- **Multi-session** — Manage multiple WhatsApp accounts simultaneously
- **QR Code & Pair Code** — Two authentication methods for linking devices
- **Rich Messages** — Text, images, video, audio, documents, stickers, location, contacts
- **Webhooks** — Real-time events with HMAC-SHA256 signature verification
- **Web Dashboard** — Visual session management interface
- **Swagger UI** — Interactive API documentation

## Installation

### Docker (Recommended)

**From Docker Hub:**
```bash
docker pull fdciabdul/wa-rs:latest
```

**Using Docker Compose:**

Create `docker-compose.yml`:
```yaml
services:
  wa-rs:
    image: fdciabdul/wa-rs:latest
    ports:
      - "3451:3451"
    environment:
      - POSTGRES_HOST=postgres
      - POSTGRES_PORT=5432
      - POSTGRES_USER=postgres
      - POSTGRES_PASSWORD=postgres
      - POSTGRES_DB=wagateway
      - JWT_SECRET=your-secret-key-change-this
    volumes:
      - wa_sessions:/app/whatsapp_sessions
    depends_on:
      - postgres

  postgres:
    image: postgres:16-alpine
    environment:
      - POSTGRES_USER=postgres
      - POSTGRES_PASSWORD=postgres
      - POSTGRES_DB=wagateway
    volumes:
      - pg_data:/var/lib/postgresql/data

volumes:
  wa_sessions:
  pg_data:
```

Run:
```bash
docker compose up -d
```

### Build from Source

**Requirements:**
- Rust Nightly
- PostgreSQL 14+

```bash
# Clone repository
git clone https://github.com/fdciabdul/wa-rs.git
cd wa-rs

# Install Rust nightly
rustup default nightly

# Configure environment
cp .env.example .env
# Edit .env with your database credentials

# Build and run
cargo run --release
```

## Authentication

### Getting the Superadmin Token

The superadmin token is generated on first startup and displayed in the server logs:

```bash
# Docker
docker compose logs wa-rs | grep "Superadmin token"

# Or check the logs output:
# [INFO] Superadmin token: eyJhbGciOiJIUzI1NiJ9...
```

You can also find it in the **Dashboard**:
1. Open http://localhost:3451/dashboard
2. Go to **Settings**
3. Copy the token from the "Superadmin Token" field

### Using the Token

Include the token in the `Authorization` header:

```bash
curl -H "Authorization: Bearer <YOUR_TOKEN>" \
  http://localhost:3451/api/v1/sessions
```

Or use the **Swagger UI** Authorize button:
1. Open http://localhost:3451/swagger-ui
2. Click **Authorize**
3. Enter your token
4. All requests will include the token automatically

## Dashboard

Access the web dashboard at: **http://localhost:3451/dashboard**

The dashboard provides:
- Session overview with connection status
- Create/delete sessions
- QR code and pair code linking
- Webhook configuration
- API settings and token access

**Public Routes** (no auth required):
- `/dashboard` — Main dashboard
- `/dashboard/sessions` — Session list
- `/dashboard/sessions/:id` — Session details
- `/dashboard/settings` — API settings

## API Usage

### Create Session

```bash
curl -X POST http://localhost:3451/api/v1/sessions \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "id": "my-session",
    "name": "My WhatsApp",
    "webhook": {
      "url": "https://example.com/webhook",
      "secret": "my-webhook-secret",
      "events": ["message", "connected", "disconnected"]
    }
  }'
```

### Connect Session (Get QR Code)

```bash
curl -X POST http://localhost:3451/api/v1/sessions/my-session/connect \
  -H "Authorization: Bearer <TOKEN>"

# Get QR code as base64 PNG
curl http://localhost:3451/api/v1/sessions/my-session/qr \
  -H "Authorization: Bearer <TOKEN>"
```

### Get Pair Code

```bash
curl -X POST http://localhost:3451/api/v1/sessions/my-session/pair-code \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"phone": "628123456789"}'
```

### Send Text Message

```bash
curl -X POST http://localhost:3451/api/v1/sessions/my-session/messages/text \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "to": "628123456789",
    "text": "Hello from WA-RS!"
  }'
```

### Send Image

```bash
curl -X POST http://localhost:3451/api/v1/sessions/my-session/messages/image \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "to": "628123456789",
    "url": "https://example.com/image.jpg",
    "caption": "Check this out!"
  }'
```

### Disconnect Session

```bash
curl -X POST http://localhost:3451/api/v1/sessions/my-session/disconnect \
  -H "Authorization: Bearer <TOKEN>"
```

### Delete Session

```bash
curl -X DELETE http://localhost:3451/api/v1/sessions/my-session \
  -H "Authorization: Bearer <TOKEN>"
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `POSTGRES_HOST` | localhost | PostgreSQL host |
| `POSTGRES_PORT` | 5432 | PostgreSQL port |
| `POSTGRES_USER` | postgres | PostgreSQL user |
| `POSTGRES_PASSWORD` | postgres | PostgreSQL password |
| `POSTGRES_DB` | wagateway | Database name |
| `JWT_SECRET` | (generated) | Token signing secret |
| `WHATSAPP_STORAGE_PATH` | ./whatsapp_sessions | Session data directory |
| `RUST_LOG` | info | Log level |

## Webhook Events

Configure webhooks to receive real-time updates:

| Event | Description |
|-------|-------------|
| `message` | Incoming message received |
| `connected` | Session connected to WhatsApp |
| `disconnected` | Session disconnected |
| `receipt` | Message delivery/read receipts |
| `presence` | Contact online/offline status |
| `qr_code` | New QR code generated |

Webhooks include HMAC-SHA256 signature in `X-Signature` header for verification.

## Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/api/v1/sessions` | List all sessions |
| POST | `/api/v1/sessions` | Create session |
| GET | `/api/v1/sessions/:id` | Get session info |
| DELETE | `/api/v1/sessions/:id` | Delete session |
| POST | `/api/v1/sessions/:id/connect` | Start connection |
| POST | `/api/v1/sessions/:id/disconnect` | Disconnect |
| GET | `/api/v1/sessions/:id/qr` | Get QR code |
| POST | `/api/v1/sessions/:id/pair-code` | Get pair code |
| POST | `/api/v1/sessions/:id/messages/text` | Send text |
| POST | `/api/v1/sessions/:id/messages/image` | Send image |
| POST | `/api/v1/sessions/:id/messages/video` | Send video |
| POST | `/api/v1/sessions/:id/messages/audio` | Send audio |
| POST | `/api/v1/sessions/:id/messages/document` | Send document |
| POST | `/api/v1/sessions/:id/messages/sticker` | Send sticker |
| POST | `/api/v1/sessions/:id/messages/location` | Send location |
| POST | `/api/v1/sessions/:id/messages/contact` | Send contact |

## License

MIT
