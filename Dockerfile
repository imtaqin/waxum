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
COPY vendor/ ./vendor/

RUN mkdir -p src && \
    echo 'fn main() { println!("dummy"); }' > src/main.rs

RUN cargo build --release 2>/dev/null || true

RUN rm -rf src target/release/waxum target/release/deps/waxum*

COPY src/ ./src/

RUN cargo build --release

RUN mkdir -p /out/app/whatsapp_sessions \
    && cp target/release/waxum /out/app/waxum

# Runtime: distroless. The binary links only glibc, libm and libgcc (TLS
# and SQLite are compiled in), so nothing else is needed: no shell, no
# package manager, no curl, no gosu. The jobs those did are in the binary
# (src/bootstrap.rs): dropping root, chowning volumes, the healthcheck.
FROM gcr.io/distroless/cc-debian12

WORKDIR /app

COPY --from=rust-builder --chown=1000:1000 /out/app /app

ENV WHATSAPP_STORAGE_PATH=/app/whatsapp_sessions
ENV RUST_LOG=waxum=info,tower_http=info
ENV HOME=/app

# The container starts as root so waxum can chown mounted volumes (which
# may still be owned by root from a pre-0.11.1 image), then it drops to
# this uid:gid before doing anything else. The gateway never runs as root.
ENV WAXUM_RUN_AS=1000:1000

EXPOSE 3451

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/app/waxum", "--healthcheck"]

CMD ["/app/waxum"]
