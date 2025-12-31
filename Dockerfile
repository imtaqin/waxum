# Build stage - use slim variant
FROM rust:slim-bookworm AS builder

WORKDIR /app

# Install build dependencies (including git for fetching from GitHub)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Copy source code
COPY . .

# Build release binary
RUN cargo build --release

# Runtime stage - minimal image
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies only
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/*

# Copy binary from builder
COPY --from=builder /app/target/release/wa-rs /app/wa-rs

# Create directory for WhatsApp session storage
RUN mkdir -p /app/whatsapp_sessions

# Environment variables
ENV POSTGRES_HOST=postgres
ENV POSTGRES_PORT=5432
ENV POSTGRES_USER=postgres
ENV POSTGRES_PASSWORD=postgres
ENV POSTGRES_DB=wagateway
ENV JWT_SECRET=change-this-in-production
ENV WHATSAPP_STORAGE_PATH=/app/whatsapp_sessions
ENV RUST_LOG=wa_rs=info,tower_http=info

EXPOSE 3451

CMD ["/app/wa-rs"]
