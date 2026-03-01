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

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies (avoid invalidating the dependency layer when only app sources change).
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && printf '%s\n' 'fn main() {}' > src/main.rs \
    && cargo build --release --locked

# Copy app sources and build the real binary.
COPY src ./src
ENV APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}
RUN rm -f target/release/codex-vibe-monitor \
    && cargo build --release --locked

# Stage 3: fetch Xray-core (xray) for forward-proxy subscription validation
# The app defaults to `XY_XRAY_BINARY=xray` (PATH lookup). If the runtime image doesn't bundle
# a real Xray-core binary, subscription validation for share links (vmess/vless/trojan/ss) fails.
FROM debian:bookworm-slim AS xray-downloader
ARG XRAY_CORE_VERSION=26.2.6
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/* \
    && case "${TARGETARCH}" in \
        amd64) XRAY_ZIP="Xray-linux-64.zip" ;; \
        arm64) XRAY_ZIP="Xray-linux-arm64-v8a.zip" ;; \
        *) echo "Unsupported TARGETARCH=${TARGETARCH} for Xray-core" >&2; exit 1 ;; \
      esac \
    && curl -fsSL -o /tmp/xray.zip "https://github.com/XTLS/Xray-core/releases/download/v${XRAY_CORE_VERSION}/${XRAY_ZIP}" \
    && unzip -q /tmp/xray.zip -d /tmp/xray \
    && install -m 0755 /tmp/xray/xray /usr/local/bin/xray \
    && install -d /usr/local/share/licenses/xray-core \
    && install -m 0644 /tmp/xray/LICENSE /usr/local/share/licenses/xray-core/LICENSE \
    && rm -rf /tmp/xray /tmp/xray.zip

# Stage 4: runtime image
FROM debian:bookworm-slim AS runtime
ARG APP_EFFECTIVE_VERSION
ARG FRONTEND_EFFECTIVE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv/app

COPY --from=rust-builder /app/target/release/codex-vibe-monitor /usr/local/bin/codex-vibe-monitor
COPY --from=xray-downloader /usr/local/bin/xray /usr/local/bin/xray
COPY --from=xray-downloader /usr/local/share/licenses/xray-core/LICENSE /usr/local/share/licenses/xray-core/LICENSE
COPY --from=web-builder /app/web/dist ./web

ENV XY_DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db \
    XY_HTTP_BIND=0.0.0.0:8080 \
    XY_STATIC_DIR=/srv/app/web \
    XY_POLL_INTERVAL_SECS=10 \
    XY_REQUEST_TIMEOUT_SECS=60 \
    APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}

LABEL org.opencontainers.image.version=${APP_EFFECTIVE_VERSION}

VOLUME ["/srv/app/data"]
EXPOSE 8080

CMD ["codex-vibe-monitor"]
