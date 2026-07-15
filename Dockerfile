FROM rust:slim-bookworm AS rust-builder

WORKDIR /app

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    curl \
    git \
    cmake \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

ENV CMAKE_POLICY_VERSION_MINIMUM=3.5

RUN rustup default nightly

COPY Cargo.toml Cargo.lock* ./

RUN mkdir -p src && \
    echo 'fn main() { println!("dummy"); }' > src/main.rs

RUN cargo build --release 2>/dev/null || true

RUN rm -rf src target/release/waxum target/release/deps/waxum*

COPY src/ ./src/

RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/*

COPY --from=rust-builder /app/target/release/waxum /app/waxum

RUN mkdir -p /app/whatsapp_sessions

ENV WHATSAPP_STORAGE_PATH=/app/whatsapp_sessions
ENV RUST_LOG=waxum=info,tower_http=info

EXPOSE 3451

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS --max-time 4 http://127.0.0.1:3451/health || exit 1

CMD ["/app/waxum"]
