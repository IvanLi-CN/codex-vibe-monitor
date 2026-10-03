# Linux debug image for hotpath-cpu (samply backend).
# See crates/hotpath/linux-dev/README.md for build/run instructions.

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        jq \
        linux-tools-generic \
        vim \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/lib/linux-tools/*/perf /usr/local/bin/perf

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal

RUN cargo install samply --locked

WORKDIR /work
CMD ["/bin/bash"]
