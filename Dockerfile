# Stage 1: build the web assets
FROM oven/bun:1.3.14-alpine AS web-builder
WORKDIR /app/web

COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile

COPY web/ ./
ARG APP_EFFECTIVE_VERSION
ENV VITE_APP_VERSION=${APP_EFFECTIVE_VERSION}
RUN bun run build

# Stage 2: build the Rust binary
# IMPORTANT: runtime image is Debian bookworm (glibc 2.36). Pin the Rust build stage to bookworm too,
# otherwise the rust:<version> default base may drift and produce a binary requiring newer GLIBC.
FROM rust:1.96.0-bookworm AS rust-builder
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
ARG APP_EFFECTIVE_VERSION
ENV APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}
RUN find src -type f -name '*.rs' -exec touch {} + \
    && rm -f target/release/codex-vibe-monitor \
    && cargo build --release --locked

# Stage 3: fetch Xray-core (xray) for forward-proxy subscription validation
# The app defaults to `XRAY_BINARY=xray` (PATH lookup). If the runtime image doesn't bundle
# a real Xray-core binary, subscription validation for share links (vmess/vless/trojan/ss) fails.
FROM debian:bookworm-slim AS xray-downloader
ARG XRAY_CORE_VERSION=26.2.6
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/* \
    && ARCH="${TARGETARCH:-$(dpkg --print-architecture)}" \
    && case "${ARCH}" in \
        amd64) XRAY_ZIP="Xray-linux-64.zip" ;; \
        arm64) XRAY_ZIP="Xray-linux-arm64-v8a.zip" ;; \
        *) echo "Unsupported TARGETARCH=${TARGETARCH} resolved_arch=${ARCH} for Xray-core" >&2; exit 1 ;; \
      esac \
    && XRAY_PRIMARY_URL="https://github.com/XTLS/Xray-core/releases/download/v${XRAY_CORE_VERSION}/${XRAY_ZIP}" \
    && XRAY_FALLBACK_URL="https://downloads.sourceforge.net/project/xray-core.mirror/v${XRAY_CORE_VERSION}/${XRAY_ZIP}" \
    && if ! curl --retry 5 --retry-all-errors --retry-delay 2 -fsSL -o /tmp/xray.zip "${XRAY_PRIMARY_URL}"; then \
         curl --retry 5 --retry-all-errors --retry-delay 2 -fsSL -o /tmp/xray.zip "${XRAY_FALLBACK_URL}"; \
       fi \
    && unzip -q /tmp/xray.zip -d /tmp/xray \
    && install -m 0755 /tmp/xray/xray /usr/local/bin/xray \
    && install -d /usr/local/share/licenses/xray-core \
    && install -m 0644 /tmp/xray/LICENSE /usr/local/share/licenses/xray-core/LICENSE \
    && rm -rf /tmp/xray /tmp/xray.zip

# Stage 4: shared runtime base
FROM debian:bookworm-slim AS runtime-base

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gzip libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv/app

COPY --from=xray-downloader /usr/local/bin/xray /usr/local/bin/xray
COPY --from=xray-downloader /usr/local/share/licenses/xray-core/LICENSE /usr/local/share/licenses/xray-core/LICENSE
COPY scripts/search-raw /usr/local/bin/search-raw

# Stage 5: production runtime image
FROM runtime-base AS production-runtime
ARG APP_EFFECTIVE_VERSION
ARG APP_GIT_REVISION
ARG FRONTEND_EFFECTIVE_VERSION

COPY --from=rust-builder /app/target/release/codex-vibe-monitor /usr/local/bin/codex-vibe-monitor
COPY --from=web-builder /app/web/dist ./web

RUN chmod 0755 /usr/local/bin/search-raw

ENV DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db \
    HTTP_BIND=0.0.0.0:8080 \
    STATIC_DIR=/srv/app/web \
    POLL_INTERVAL_SECS=10 \
    REQUEST_TIMEOUT_SECS=60 \
    MALLOC_ARENA_MAX=8 \
    APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}

LABEL org.opencontainers.image.version=${APP_EFFECTIVE_VERSION} \
      org.opencontainers.image.revision=${APP_GIT_REVISION}

VOLUME ["/srv/app/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=6 CMD curl --fail --silent http://127.0.0.1:8080/health || exit 1

CMD ["codex-vibe-monitor"]

# Stage 6: PR-only smoke image. The workflow produces the binary, web bundle, and
# Xray archive outside Docker so this target exercises the runtime without repeating
# the release compiler or Xray downloader pipelines. GitHub-hosted runners use Ubuntu
# 24.04; matching its glibc here avoids executing a host-built debug binary against
# the older production runtime.
FROM ubuntu:24.04 AS ci-smoke-runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gzip libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv/app

COPY .ci-smoke/xray /usr/local/bin/xray
COPY .ci-smoke/xray.LICENSE /usr/local/share/licenses/xray-core/LICENSE
COPY scripts/search-raw /usr/local/bin/search-raw
COPY .ci-smoke/codex-vibe-monitor /usr/local/bin/codex-vibe-monitor
COPY .ci-smoke/web ./web

RUN chmod 0755 /usr/local/bin/search-raw /usr/local/bin/codex-vibe-monitor

ARG APP_EFFECTIVE_VERSION
ENV DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db \
    HTTP_BIND=0.0.0.0:8080 \
    STATIC_DIR=/srv/app/web \
    POLL_INTERVAL_SECS=10 \
    REQUEST_TIMEOUT_SECS=60 \
    MALLOC_ARENA_MAX=8 \
    APP_EFFECTIVE_VERSION=${APP_EFFECTIVE_VERSION}

LABEL org.opencontainers.image.version=${APP_EFFECTIVE_VERSION}

VOLUME ["/srv/app/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=6 CMD curl --fail --silent http://127.0.0.1:8080/health || exit 1

CMD ["codex-vibe-monitor"]

# Stage 7: project-owned backend test environment. This target is intentionally
# separate from the production image so test tooling and writable build paths
# cannot alter the release runtime contract.
FROM rust:1.96.0-bookworm AS backend-test
ARG CARGO_NEXTEST_VERSION=0.9.138
ARG CARGO_NEXTEST_SHA256_AMD64=3793bf0c27607b196f502c39b2108f571de89fcda7586ae6beefa11ee177b216

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl pkg-config libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy \
    && curl --retry 5 --retry-all-errors --retry-delay 2 -fsSL \
      -o /tmp/cargo-nextest.tar.gz \
      "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${CARGO_NEXTEST_VERSION}/cargo-nextest-${CARGO_NEXTEST_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
    && echo "${CARGO_NEXTEST_SHA256_AMD64}  /tmp/cargo-nextest.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/cargo-nextest.tar.gz -C /tmp \
    && install -m 0755 /tmp/cargo-nextest /usr/local/cargo/bin/cargo-nextest \
    && rm -f /tmp/cargo-nextest.tar.gz /tmp/cargo-nextest

WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY scripts/search-raw ./scripts/search-raw
COPY .github/scripts/run-backend-tests.sh ./.github/scripts/run-backend-tests.sh
RUN mkdir -p target && chown 65534:65534 target

ENV BACKEND_TEST_WORKSPACE=/tmp/codex-vibe-monitor-backend-test \
    CARGO_TARGET_DIR=/tmp/codex-vibe-monitor-backend-test/target \
    RUST_MIN_STACK=8388608

ENTRYPOINT ["bash", ".github/scripts/run-backend-tests.sh"]

# Stage 8: retain the production image as the default Docker build target.
FROM production-runtime AS runtime
