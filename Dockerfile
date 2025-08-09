# syntax=docker/dockerfile:1

# ---------- Builder ----------
FROM rust:1-bookworm AS builder

# System deps for building jq-rs and TLS
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    make \
    jq \
    libjq-dev \
    libonig-dev \
    libssl-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Leverage build cache
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build release binary
RUN cargo build --release && strip target/release/webpipe

# ---------- Runtime ----------
FROM ubuntu:22.04

# Runtime libs: libjq + oniguruma for jq-rs, OpenSSL for reqwest TLS, and certs
RUN apt-get update && apt-get install -y --no-install-recommends \
    libjq1 \
    libonig5 \
    libssl3 \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN groupadd -r wp && useradd -r -g wp -d /app wp

# App layout and defaults
WORKDIR /app
RUN mkdir -p /app/public /app/scripts && chown -R wp:wp /app
ENV WEBPIPE_PUBLIC_DIR=/app/public \
    WEBPIPE_SCRIPTS_DIR=/app/scripts \
    RUST_LOG=info \
    PORT=8080

# Copy binary
COPY --from=builder /build/target/release/webpipe /usr/local/bin/webpipe

USER wp
EXPOSE 8080

# Run like: docker run ... ghcr.io/williamcotton/webpipe:latest app.wp
ENTRYPOINT ["webpipe"]
CMD ["--help"]