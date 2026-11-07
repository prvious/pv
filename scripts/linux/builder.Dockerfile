FROM ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401

ARG TARGETARCH
ENV CARGO_HOME=/opt/cargo \
    RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:$PATH

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        bison build-essential ca-certificates cmake curl file flex git jq libaio-dev \
        libncurses-dev patchelf pkg-config python3 unzip xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain stable \
    && rm /tmp/rustup.sh \
    && case "$TARGETARCH" in \
        amd64) nextest_platform=linux ;; \
        arm64) nextest_platform=linux-arm ;; \
        *) exit 1 ;; \
    esac \
    && curl -LsSf "https://get.nexte.st/0.9.146/$nextest_platform" \
        -o /tmp/nextest.tar.gz \
    && tar -xzf /tmp/nextest.tar.gz -C /opt/cargo/bin cargo-nextest \
    && rm /tmp/nextest.tar.gz

ARG BUILDER_UID=1000
RUN useradd --create-home --uid "$BUILDER_UID" dev \
    && mkdir /work /target \
    && chown -R dev:dev /opt/cargo /work /target
USER dev
WORKDIR /work
