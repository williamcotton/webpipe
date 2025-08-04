# Docker Container Plan for Web Pipe (wp)

## Overview

This plan outlines the strategy for shipping Web Pipe (wp) as a Docker container that can be used as `FROM some-github-docker-registry-for-wp`. The container will include a pre-built wp runtime with all middleware and dependencies.

## Container Architecture

### Base Image Strategy
- **Primary Base**: `ubuntu:22.04` (for broad compatibility and dependency availability)
- **Alternative Base**: `debian:bookworm-slim` (smaller footprint option)
- **Multi-stage build** to minimize final image size

### Dependencies Analysis

#### System Dependencies (From GitHub Actions)
```dockerfile
# Core build tools
clang                 # Primary compiler (not gcc)
make                  # Build system
pkg-config            # Build configuration

# Required libraries for runtime
libmicrohttpd12       # HTTP server library
libjansson4           # JSON processing
libjq1                # jq JSON queries  
liblua5.4-0           # Lua scripting
libpq5                # PostgreSQL client
libargon2-1           # Password hashing (for auth middleware)
libcurl4              # HTTP client for fetch middleware
libbsd0               # BSD compatibility library
```

#### Build Dependencies (Multi-stage)
```dockerfile
# Build-time only dependencies (from test.yml)
clang                 # Primary compiler
libmicrohttpd-dev     # HTTP server development headers
libpq-dev             # PostgreSQL development headers
libjansson-dev        # JSON processing development headers
libjq-dev             # jq development headers
liblua5.4-dev         # Lua development headers
libcurl4-openssl-dev  # cURL development headers with OpenSSL
libbsd-dev            # BSD compatibility development headers
libargon2-dev         # Argon2 password hashing development headers
postgresql-client     # PostgreSQL client tools (for testing)
valgrind              # Memory debugging tool (for leak detection)
```

### Container Structure

```
/usr/local/bin/wp              # Main wp executable
/usr/local/lib/wp/             # Middleware directory
├── auth.so
├── cache.so
├── debug.so
├── fetch.so
├── jq.so
├── log.so
├── lua.so
├── mustache.so
├── pg.so
└── validate.so
/usr/local/include/            # API header for extensions
├── webpipe-middleware-api.h
/opt/wp/scripts/               # Default Lua scripts
├── dateFormatter.lua
├── querybuilder.lua
└── contentful.lua
/app/                          # Default working directory for user .wp files
```

## Dockerfile Strategy

### Multi-Stage Build

```dockerfile
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

# Copy built artifacts
COPY --from=builder /build/build/wp /usr/local/bin/wp
COPY --from=builder /build/middleware/*.so /usr/local/lib/wp/
COPY --from=builder /build/include/webpipe-middleware-api.h /usr/local/include/
COPY --from=builder /build/scripts/ /opt/wp/scripts/

# Set up environment
ENV WP_MIDDLEWARE_DIR=/usr/local/lib/wp
ENV WP_SCRIPTS_DIR=/opt/wp/scripts
WORKDIR /app

# Expose default port
EXPOSE 8080

# Default command
ENTRYPOINT ["/usr/local/bin/wp"]
CMD ["--help"]
```

## Usage Patterns

### Basic Usage
```dockerfile
FROM ghcr.io/your-org/webpipe:latest

# Copy your wp files
COPY . /app/

# Run your application
CMD ["app.wp", "--port", "8080"]
```

### With Custom Configuration
```dockerfile
FROM ghcr.io/your-org/webpipe:latest

# Copy application files
COPY . /app/
COPY .env.production .env

# Expose custom port
EXPOSE 3000

CMD ["api.wp", "--port", "3000"]
```

### Development Mode
```dockerfile
FROM ghcr.io/your-org/webpipe:latest

# Install development tools
RUN apt-get update && apt-get install -y \
    valgrind \
    gdb \
    && rm -rf /var/lib/apt/lists/*

# Copy source for live development
VOLUME ["/app"]

CMD ["--help"]
```

## Build Optimization

### Image Size Optimization
- Multi-stage build removes build dependencies (~200MB savings)
- Strip debug symbols from binaries
- Use .dockerignore to exclude unnecessary files
- Compress middleware .so files if beneficial

### Build Performance
- Layer caching for dependencies
- Parallel builds where possible
- Pre-built base images with common dependencies

### Security Considerations
- Non-root user execution
- Minimal attack surface (only runtime dependencies)
- Regular base image updates
- Vulnerability scanning in CI

## CI/CD Integration

### GitHub Actions Workflow

```yaml
name: Build and Push Docker Image

on:
  push:
    branches: [main, contentful]  # Match existing test triggers
    tags: ['v*']
  pull_request:
    branches: [main]

jobs:
  # Run tests first (based on existing test.yml)
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:14
        env:
          POSTGRES_PASSWORD: postgres
          POSTGRES_USER: postgres
          POSTGRES_DB: wp-test
        ports:
          - 5432:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v4
      
      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            clang \
            libmicrohttpd-dev \
            libpq-dev \
            libjansson-dev \
            libjq-dev \
            liblua5.4-dev \
            libcurl4-openssl-dev \
            postgresql-client \
            libbsd-dev \
            valgrind \
            libargon2-dev

      - name: Build application
        run: make all

      - name: Install middleware
        run: make install-middleware

      - name: Run static analysis
        run: make test-analyze

      - name: Run linting
        run: make test-lint

      - name: Run all tests
        run: make test
        env:
          WP_PG_HOST: localhost
          WP_PG_USER: postgres
          WP_PG_PASSWORD: postgres
          WP_PG_DATABASE: wp-test

  # Build Docker image only after tests pass
  build:
    needs: test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3
        
      - name: Login to GitHub Container Registry
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
          
      - name: Extract metadata
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ghcr.io/${{ github.repository }}
          tags: |
            type=ref,event=branch
            type=ref,event=pr
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            
      - name: Build and push
        uses: docker/build-push-action@v5
        with:
          context: .
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
```

## Testing Strategy

### Container Testing (Based on Existing Test Suite)
- Test container builds successfully with exact CI dependencies
- Verify all middleware (.so files) load correctly
- Test with sample .wp files (test.wp, blog.wp)
- Run full test suite in container (unit, integration, system)
- Memory leak detection with valgrind
- Performance benchmarks
- Security scanning

### Runtime Testing
```bash
# Test basic functionality
docker run --rm ghcr.io/your-org/webpipe:latest --help

# Test with sample app (using existing test.wp)
docker run -p 8080:8080 -v $(pwd)/test.wp:/app/test.wp \
  ghcr.io/your-org/webpipe:latest test.wp

# Test BDD suite (as per CI)
docker run --rm ghcr.io/your-org/webpipe:latest test.wp --test

# Test middleware loading
docker run --rm -e WP_MIDDLEWARE_DIR=/usr/local/lib/wp \
  ghcr.io/your-org/webpipe:latest test.wp

# Test with PostgreSQL (similar to CI setup)
docker run --network host \
  -e WP_PG_HOST=localhost \
  -e WP_PG_USER=postgres \
  -e WP_PG_PASSWORD=postgres \
  -e WP_PG_DATABASE=wp-test \
  ghcr.io/your-org/webpipe:latest test.wp
```

## Registry Strategy

### GitHub Container Registry (ghcr.io)
- **Main registry**: `ghcr.io/your-org/webpipe`
- **Tags**: 
  - `latest` (main branch)
  - `v1.0.0` (semantic versions)
  - `main` (latest main branch)
  - `pr-123` (pull requests)

### Image Variants
- `ghcr.io/your-org/webpipe:latest` - Standard runtime
- `ghcr.io/your-org/webpipe:debug` - With debug tools (valgrind, gdb)
- `ghcr.io/your-org/webpipe:alpine` - Alpine-based (smaller, if dependencies allow)
- `ghcr.io/your-org/webpipe:dev` - Development image with build tools

## Documentation and Examples

### README Updates
- Add Docker usage section
- Include docker-compose examples
- Document environment variables
- Provide troubleshooting guide

### Example Applications
- Simple "hello world" wp app (based on existing test.wp)
- Todo application with authentication (from existing test.wp)
- Multi-service application with PostgreSQL
- Blog application (using existing blog.wp)
- Production deployment guide
- Custom middleware development

## Migration Path

### Phase 1: Local Development
- Create working Dockerfile
- Build and test containers locally
- Validate all middleware loads correctly
- Test with existing .wp files (test.wp, blog.wp)
- Ensure compatibility with local development workflow

### Phase 2: GitHub Registry Integration
- Set up CI/CD pipeline for automated builds
- Configure GitHub Container Registry (ghcr.io)
- Multi-architecture builds (amd64/arm64)
- Automated testing in CI

### Phase 3: Optimization & Ecosystem
- Multi-stage optimization for smaller images
- Advanced caching strategies
- Docker Compose templates
- Kubernetes manifests
- Helm charts
- Integration examples

## Environment Variables

```bash
# Core configuration
WP_PORT=8080                    # Server port
WP_MIDDLEWARE_DIR=/usr/local/lib/wp  # Middleware directory
WP_SCRIPTS_DIR=/opt/wp/scripts  # Lua scripts directory

# Database configuration (pg middleware)
WP_PG_HOST=localhost
WP_PG_PORT=5432
WP_PG_DATABASE=myapp
WP_PG_USER=postgres
WP_PG_PASSWORD=password

# Optional configuration
WP_LOG_LEVEL=info
WP_CACHE_ENABLED=true
WP_AUTH_SESSION_TTL=604800
```

## Key Insights from GitHub Actions Analysis

### Critical Dependencies Confirmed
- **clang** (not gcc) is the primary compiler
- **libbsd-dev** is required (BSD compatibility)
- **valgrind** needed for memory leak detection
- **postgresql-client** for database testing
- Exact same dependency list as CI ensures compatibility

### Build Process Alignment
- Use `make all` (not separate make && make middleware)
- Static analysis with `make test-analyze`
- Linting with `make test-lint`
- Full test suite with `make test`
- Memory leak detection integrated

### Testing Integration
- PostgreSQL 14 service for database tests
- Comprehensive test matrix (unit, integration, system)
- BDD test suite support
- Cross-platform testing (Ubuntu + macOS patterns)

## Success Metrics

- Container builds in under 2 minutes (matching CI performance)
- Final image size under 100MB
- Zero critical vulnerabilities
- Support for both amd64 and arm64
- 99% CI success rate (matching existing test.yml)
- All middleware load correctly (.so files)
- Comprehensive documentation
- Full test suite passes in container

This plan provides a complete strategy for containerizing wp with exact dependency matching to the proven GitHub Actions CI/CD pipeline, ensuring compatibility and reliability.