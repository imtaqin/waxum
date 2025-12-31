# WhatsApp REST API

A multi-session REST API gateway for WhatsApp Web client built with Rust.

## Features

- Multi-session support
- QR Code & Pair Code authentication
- Send messages (text, image, video, audio, document, sticker, location, contact)
- Webhook support with HMAC-SHA256 signatures
- JWT authentication
- PostgreSQL database
- Swagger UI documentation

## Quick Start

### Using Docker Compose

```bash
docker compose up -d
```

### Manual Setup

1. **Requirements**
   - Rust 1.75+
   - PostgreSQL 14+

2. **Configure environment**
   ```bash
   cp .env.example .env
   # Edit .env with your settings
   ```

3. **Run**
   ```bash
   cargo run --release
   ```

4. **Access**
   - API: http://localhost:3000
   - Swagger UI: http://localhost:3000/swagger-ui

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `POSTGRES_HOST` | localhost | PostgreSQL host |
| `POSTGRES_PORT` | 5432 | PostgreSQL port |
| `POSTGRES_USER` | postgres | PostgreSQL user |
| `POSTGRES_PASSWORD` | postgres | PostgreSQL password |
| `POSTGRES_DB` | wagateway | PostgreSQL database |
| `JWT_SECRET` | - | JWT signing secret |
| `WHATSAPP_STORAGE_PATH` | ./whatsapp_sessions | Session storage path |

## API Usage

### Authentication

All endpoints require JWT token:
```bash
curl -H "Authorization: Bearer <TOKEN>" http://localhost:3000/api/v1/sessions
```

### Create Session

```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "id": "my-session",
    "name": "My Account",
    "webhook": {
      "url": "https://example.com/webhook",
      "events": ["message", "connected"]
    }
  }'
```

### Get QR Code

```bash
curl http://localhost:3000/api/v1/sessions/my-session/qr \
  -H "Authorization: Bearer <TOKEN>"
```

### Send Message

```bash
curl -X POST http://localhost:3000/api/v1/sessions/my-session/messages/text \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "to": "628123456789",
    "text": "Hello from API!"
  }'
```

## License

MIT
