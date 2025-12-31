# Build stage - use slim variant
FROM rust:slim-bookworm AS builder

WORKDIR /workspace

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace
COPY . .

WORKDIR /workspace/rest-api

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
COPY --from=builder /workspace/target/release/whatsapp-rest-api /app/whatsapp-rest-api

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
ENV RUST_LOG=whatsapp_rest_api=info,tower_http=info

EXPOSE 3000

CMD ["/app/whatsapp-rest-api"]
