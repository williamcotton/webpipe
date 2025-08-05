# Stage 1: Build environment
FROM ubuntu:22.04 AS builder

# Install build dependencies (based on GitHub Actions test.yml)
RUN apt-get update && apt-get install -y \
    clang \
    make \
    pkg-config \
    libmicrohttpd-dev \
    libpq-dev \
    libjansson-dev \
    libjq-dev \
    liblua5.4-dev \
    libcurl4-openssl-dev \
    libbsd-dev \
    libargon2-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Copy source code
COPY . /build
WORKDIR /build

# Build wp and middleware (following exact CI process)
RUN make clean && \
    make all && \
    make install-middleware

# Stage 2: Runtime environment  
FROM ubuntu:22.04

# Install runtime dependencies only
RUN apt-get update && apt-get install -y \
    libmicrohttpd12 \
    libjansson4 \
    libjq1 \
    liblua5.4-0 \
    libpq5 \
    libargon2-1 \
    libcurl4 \
    libbsd0 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN groupadd -r wp && useradd -r -g wp wp

# Copy built artifacts
COPY --from=builder /build/build/wp /usr/local/bin/wp
COPY --from=builder /build/include/webpipe-middleware-api.h /usr/local/include/
COPY --from=builder /build/scripts/ /opt/wp/scripts/

# Set up environment
ENV WP_SCRIPTS_DIR=/opt/wp/scripts

# Create app directory and middleware directory
RUN mkdir -p /app /app/middleware && chown -R wp:wp /app
WORKDIR /app

# Copy middleware to expected location (./middleware/ relative to working directory)
COPY --from=builder /build/middleware/*.so ./middleware/

# Switch to non-root user
USER wp

# Expose default port
EXPOSE 8080

# Default command
ENTRYPOINT ["/usr/local/bin/wp"]
CMD ["--help"]