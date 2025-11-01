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
FROM rust:1.86 AS rust-builder
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

# Stage 3: runtime image
FROM debian:bookworm-slim AS runtime
ARG APP_EFFECTIVE_VERSION
ARG FRONTEND_EFFECTIVE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv/app

COPY --from=rust-builder /app/target/release/codex-vibe-monitor /usr/local/bin/codex-vibe-monitor
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
