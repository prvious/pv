ARG BASE=ubuntu:22.04
FROM ${BASE}

ENV container=docker
RUN if command -v apt-get >/dev/null; then \
        apt-get update \
        && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
            binutils ca-certificates curl dbus dbus-user-session file iproute2 libnss3-tools \
            libpam-systemd procps python3 sudo systemd systemd-sysv \
        && if [ ! -x /lib/systemd/systemd-resolved ]; then \
            DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends systemd-resolved; \
        fi \
        && rm -rf /var/lib/apt/lists/*; \
    elif command -v dnf >/dev/null; then \
        dnf install -y binutils ca-certificates curl dbus-daemon file iproute NetworkManager \
            nss-tools procps-ng python3 sudo systemd systemd-pam systemd-resolved \
        && dnf clean all; \
    else \
        exit 1; \
    fi

RUN if command -v systemctl >/dev/null; then \
        existing_user="$(getent passwd 1000 | cut -d: -f1)"; \
        if [ -n "$existing_user" ]; then userdel "$existing_user"; fi \
        && useradd --create-home --uid 1000 dev \
        && printf 'dev ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/dev \
        && chmod 0440 /etc/sudoers.d/dev \
        && systemctl enable systemd-resolved; \
    fi

STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
