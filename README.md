

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

# Instalation & Documentation 

Open this : [Documentation](https://wa-rs.imtaqin.id/)

## License

MIT
