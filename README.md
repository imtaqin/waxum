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

Production-grade.

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

## Docs

Everything else — endpoints, webhooks, health probes, deployment,
`.env` reference — lives in the docs:

**[waxum.imtaqin.id](https://waxum.imtaqin.id)**

## License

MIT
