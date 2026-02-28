# Stage 1: build the web assets
FROM node:20-alpine AS web-builder
ARG APP_EFFECTIVE_VERSION
WORKDIR /app/web

COPY web/package*.json ./
RUN npm ci

COPY web/ ./
ENV VITE_APP_VERSION=${APP_EFFECTIVE_VERSION}
RUN npm run build

# Stage 2: build the Rust binary
# IMPORTANT: runtime image is Debian bookworm (glibc 2.36). Pin the Rust build stage to bookworm too,
# otherwise the rust:<version> default base may drift and produce a binary requiring newer GLIBC.
FROM rust:1.91.0-bookworm AS rust-builder
ARG APP_EFFECTIVE_VERSION
WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libsqlite3-dev \
    && cargo fetch

# Copy remaining sources and build
COPY . .
ENV APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}
RUN cargo build --release

# Stage 3: fetch Xray binary (for vmess/vless/trojan/ss forward proxy validation)
FROM debian:bookworm-slim AS xray-builder
ARG XRAY_VERSION=26.2.6
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /tmp/xray

RUN set -eux; \
    mkdir -p /out; \
    case "${TARGETARCH:-}" in \
      amd64) machine="64" ;; \
      arm64) machine="arm64-v8a" ;; \
      "") \
        arch="$(dpkg --print-architecture || true)"; \
        case "$arch" in \
          amd64) machine="64" ;; \
          arm64) machine="arm64-v8a" ;; \
          *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
        esac ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    base="https://github.com/XTLS/Xray-core/releases/download/v${XRAY_VERSION}"; \
    zip="Xray-linux-${machine}.zip"; \
    curl -fsSLo "${zip}" "${base}/${zip}"; \
    curl -fsSLo "${zip}.dgst" "${base}/${zip}.dgst"; \
    sha="$(grep -E '^SHA2-256=' "${zip}.dgst" | head -n1 | cut -d= -f2 | tr -d '[:space:]')"; \
    if [ -z "${sha}" ]; then \
      sha="$(grep -E '^sha256:' "${zip}.dgst" | head -n1 | cut -d: -f2 | tr -d '[:space:]')"; \
    fi; \
    test -n "${sha}"; \
    echo "${sha}  ${zip}" | sha256sum -c -; \
    unzip -q "${zip}"; \
    install -m 0755 xray /out/xray; \
    curl -fsSLo /out/LICENSE "https://raw.githubusercontent.com/XTLS/Xray-core/v${XRAY_VERSION}/LICENSE"

# Stage 4: runtime image
FROM debian:bookworm-slim AS runtime
ARG APP_EFFECTIVE_VERSION
ARG FRONTEND_EFFECTIVE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv/app

COPY --from=rust-builder /app/target/release/codex-vibe-monitor /usr/local/bin/codex-vibe-monitor
COPY --from=xray-builder /out/xray /usr/local/bin/xray
COPY --from=xray-builder /out/LICENSE /usr/local/share/licenses/xray-core/LICENSE
COPY --from=web-builder /app/web/dist ./web

ENV XY_DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db \
    XY_HTTP_BIND=0.0.0.0:8080 \
    XY_STATIC_DIR=/srv/app/web \
    XY_POLL_INTERVAL_SECS=10 \
    XY_REQUEST_TIMEOUT_SECS=60 \
    XY_XRAY_BINARY=/usr/local/bin/xray \
    APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}

LABEL org.opencontainers.image.version=${APP_EFFECTIVE_VERSION}

VOLUME ["/srv/app/data"]
EXPOSE 8080

CMD ["codex-vibe-monitor"]
