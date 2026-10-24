FROM ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401

ARG TARGETARCH
ENV CARGO_HOME=/opt/cargo \
    RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:$PATH

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        bison build-essential ca-certificates cmake curl file git libaio-dev \
        patchelf pkg-config python3 unzip xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain stable \
        --component clippy --component rustfmt \
    && rm /tmp/rustup.sh \
    && case "$TARGETARCH" in \
        amd64) nextest_platform=linux ;; \
        arm64) nextest_platform=linux-arm ;; \
        *) exit 1 ;; \
    esac \
    && curl -LsSf "https://get.nexte.st/0.9.146/$nextest_platform" \
        -o /tmp/nextest.tar.gz \
    && tar -xzf /tmp/nextest.tar.gz -C /opt/cargo/bin cargo-nextest \
    && rm /tmp/nextest.tar.gz \
    && cargo install --locked cargo-shear \
    && rm -rf /opt/cargo/registry /opt/cargo/git

RUN case "$TARGETARCH" in \
        amd64) shellcheck_arch=x86_64; shellcheck_sha=8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198 ;; \
        arm64) shellcheck_arch=aarch64; shellcheck_sha=12b331c1d2db6b9eb13cfca64306b1b157a86eb69db83023e261eaa7e7c14588 ;; \
        *) exit 1 ;; \
    esac \
    && curl -LsSf "https://github.com/koalaman/shellcheck/releases/download/v0.11.0/shellcheck-v0.11.0.linux.$shellcheck_arch.tar.xz" -o /tmp/shellcheck.tar.xz \
    && printf '%s  /tmp/shellcheck.tar.xz\n' "$shellcheck_sha" | sha256sum -c - \
    && tar -xJf /tmp/shellcheck.tar.xz -C /tmp \
    && install /tmp/shellcheck-v0.11.0/shellcheck /usr/local/bin/shellcheck \
    && rm -rf /tmp/shellcheck.tar.xz /tmp/shellcheck-v0.11.0

ARG BUILDER_UID=1000
RUN useradd --create-home --uid "$BUILDER_UID" dev \
    && mkdir /work /target \
    && chown -R dev:dev /opt/cargo /work /target
USER dev
WORKDIR /work
