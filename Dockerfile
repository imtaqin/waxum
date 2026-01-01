# Rust build stage
FROM rust:slim-bookworm AS rust-builder

WORKDIR /app

# Install build dependencies (including git for fetching from GitHub)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust nightly (required by whatsapp-rust for portable_simd)
RUN rustup default nightly

# === DEPENDENCY CACHING LAYER ===
# Copy only dependency manifests and askama config first
COPY Cargo.toml Cargo.lock* askama.toml ./

# Create dummy source files to build dependencies only
RUN mkdir -p src && \
    echo 'fn main() { println!("dummy"); }' > src/main.rs

# Create dummy templates directory (Askama checks at compile time)
RUN mkdir -p templates && \
    echo '<!DOCTYPE html><html><body></body></html>' > templates/base.askama

# Build dependencies only (this layer will be cached)
RUN cargo build --release 2>/dev/null || true

# Remove dummy source AND the dummy binary (important!)
RUN rm -rf src templates target/release/wa-rs target/release/deps/wa_rs*

# === ACTUAL SOURCE BUILD ===
# Now copy real source code and templates
COPY src/ ./src/
COPY templates/ ./templates/

# Build release binary (dependencies are already cached)
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

# Copy binary from builder (templates are embedded at compile time)
COPY --from=rust-builder /app/target/release/wa-rs /app/wa-rs

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
