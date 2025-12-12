# syntax=docker/dockerfile:1

# 1. Import cross-compilation helpers
FROM --platform=$BUILDPLATFORM tonistiigi/xx AS xx

# ---------- Builder ----------
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS builder

# Copy xx scripts
COPY --from=xx / /

# 2. Install NATIVE build tools (Runs on x86_64)
# We need clang (compiler), lld (linker), and pkg-config (to find libs).
RUN apt-get update && apt-get install -y --no-install-recommends \
  clang \
  lld \
  file \
  git \
  pkg-config \
  && rm -rf /var/lib/apt/lists/*

# 3. Install TARGET architecture libraries
# - libssl-dev: required for openssl-sys
# - libc6-dev: required for C headers (fixes 'bits/libc-header-start.h' error)
# - libgcc-12-dev: required for C runtime files like crtbeginS.o (fixes linker errors)
# Note: We install 'libgcc-12-dev' specifically instead of 'gcc' to avoid
# pulling in the full compiler which caused header conflicts in earlier attempts.
ARG TARGETPLATFORM
RUN xx-apt-get update && xx-apt-get install -y --no-install-recommends \
  libssl-dev \
  libc6-dev \
  libgcc-12-dev

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# 4. Build using xx-cargo
# We set PKG_CONFIG env vars to ensure it finds the ARM64 libraries installed above.
RUN PKG_CONFIG_ALLOW_CROSS=1 \
  PKG_CONFIG_PATH=/usr/lib/$(xx-info triple)/pkgconfig \
  xx-cargo build --release && \
  cp target/$(xx-cargo --print-target-triple)/release/webpipe /build/webpipe && \
  xx-verify /build/webpipe

# ---------- Runtime ----------
FROM ubuntu:22.04

# Runtime libs
RUN apt-get update && apt-get install -y --no-install-recommends \
  libssl3 \
  ca-certificates \
  && rm -rf /var/lib/apt/lists/*

RUN groupadd -r wp && useradd -r -g wp -d /app wp

WORKDIR /app
RUN mkdir -p /app/public /app/scripts && chown -R wp:wp /app
ENV WEBPIPE_PUBLIC_DIR=/app/public \
  WEBPIPE_SCRIPTS_DIR=/app/scripts \
  RUST_LOG=info \
  PORT=8080

COPY --from=builder /build/webpipe /usr/local/bin/webpipe

USER wp
EXPOSE 8080

ENTRYPOINT ["webpipe"]
CMD ["--help"]