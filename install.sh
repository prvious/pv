#!/usr/bin/env bash
set -euo pipefail

# ── Colors ───────────────────────────────────────────────────────────────────

PURPLE='\033[38;5;141m'
MUTED='\033[0;2m'
RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

# ── Usage ────────────────────────────────────────────────────────────────────

usage() {
    cat <<EOF
pv installer

Usage: install.sh [options]

Options:
    -h, --help              Display this help message
    -v, --version [version] Install a specific pv version (e.g., 0.1.0)
    --install-dir <path>    Where to install the pv binary (default: ~/.local/bin)
    --php [version]         PHP version to install (e.g., 8.4). Auto-detects if omitted.
    --tld <tld>             Top-level domain for local sites (default: test)
    --no-modify-path        Don't modify shell config files (.zshrc, .bashrc, etc.)

Examples:
    curl -fsSL https://pv.prvious.dev/install | bash
    curl -fsSL https://pv.prvious.dev/install | bash -s -- --php 8.4
    curl -fsSL https://pv.prvious.dev/install | bash -s -- --version 0.2.0
    curl -fsSL https://pv.prvious.dev/install | bash -s -- --install-dir /usr/local/bin
EOF
}

# ── Parse args ───────────────────────────────────────────────────────────────

requested_version=""
no_modify_path=false
php_version=""
tld=""
install_dir=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        -v|--version)
            if [[ -n "${2:-}" ]]; then
                requested_version="$2"
                shift 2
            else
                echo -e "${RED}Error: --version requires a version argument${NC}"
                exit 1
            fi
            ;;
        --php)
            if [[ -n "${2:-}" ]]; then
                php_version="$2"
                shift 2
            else
                echo -e "${RED}Error: --php requires a version argument${NC}"
                exit 1
            fi
            ;;
        --tld)
            if [[ -n "${2:-}" ]]; then
                tld="$2"
                shift 2
            else
                echo -e "${RED}Error: --tld requires a value${NC}"
                exit 1
            fi
            ;;
        --install-dir)
            if [[ -n "${2:-}" ]]; then
                install_dir="$2"
                shift 2
            else
                echo -e "${RED}Error: --install-dir requires a path${NC}"
                exit 1
            fi
            ;;
        --no-modify-path)
            no_modify_path=true
            shift
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}" >&2
            exit 1
            ;;
    esac
done

# ── Platform detection ───────────────────────────────────────────────────────

detect_platform() {
    local os
    os=$(uname -s)

    if [[ "$os" != "Darwin" ]]; then
        echo ""
        echo -e "${RED}  pv currently only supports macOS.${NC}"
        echo -e "${MUTED}  Linux support is planned. Follow https://github.com/prvious/pv for updates.${NC}"
        echo ""
        exit 1
    fi

    local arch
    arch=$(uname -m)

    # Rosetta detection: catch x86 shells running on ARM Macs
    if [[ "$arch" == "x86_64" ]]; then
        local rosetta
        rosetta=$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)
        if [[ "$rosetta" == "1" ]]; then
            arch="arm64"
        fi
    fi

    case "$arch" in
        arm64)  echo "darwin-arm64" ;;
        x86_64) echo "darwin-amd64" ;;
        *)
            echo -e "${RED}  Unsupported architecture: $arch${NC}"
            exit 1
            ;;
    esac
}

# ── Progress bar ─────────────────────────────────────────────────────────────

unbuffered_sed() {
    if echo | sed -u -e "" >/dev/null 2>&1; then
        sed -nu "$@"
    elif echo | sed -l -e "" >/dev/null 2>&1; then
        sed -nl "$@"
    else
        local pad
        pad="$(printf "\n%512s" "")"
        sed -ne "s/$/\\${pad}/" "$@"
    fi
}

print_progress() {
    local bytes="$1"
    local length="$2"
    [[ "$length" -gt 0 ]] || return 0

    local width=40
    local percent=$(( bytes * 100 / length ))
    [[ "$percent" -gt 100 ]] && percent=100
    local on=$(( percent * width / 100 ))
    local off=$(( width - on ))

    local filled
    filled=$(printf "%*s" "$on" "")
    filled=${filled// /■}
    local empty
    empty=$(printf "%*s" "$off" "")
    empty=${empty// /･}

    printf "\r  ${PURPLE}%s%s %3d%%${NC}" "$filled" "$empty" "$percent" >&4
}

download_with_progress() {
    local url="$1"
    local output="$2"

    if [[ -t 2 ]]; then
        exec 4>&2
    else
        exec 4>/dev/null
    fi

    local tmp_dir="${TMPDIR:-/tmp}"
    local basename="${tmp_dir}/pv_install_$$"
    local tracefile="${basename}.trace"

    rm -f "$tracefile"
    mkfifo "$tracefile"

    # Hide cursor
    printf "\033[?25l" >&4

    trap "trap - RETURN; rm -f \"$tracefile\"; printf '\033[?25h' >&4; exec 4>&-" RETURN

    (
        curl --trace-ascii "$tracefile" -fsL -o "$output" "$url"
    ) &
    local curl_pid=$!

    # Kill background curl on script exit (Ctrl+C, set -e, etc.)
    trap "kill $curl_pid 2>/dev/null; rm -f \"$tracefile\"; printf '\033[?25h' >&4" EXIT

    unbuffered_sed \
        -e 'y/ACDEGHLNORTV/acdeghlnortv/' \
        -e '/^0000: content-length:/p' \
        -e '/^<= recv data/p' \
        "$tracefile" | \
    {
        local length=0
        local bytes=0

        while IFS=" " read -r -a line; do
            [[ "${#line[@]}" -lt 2 ]] && continue
            local tag="${line[0]} ${line[1]}"

            if [[ "$tag" == "0000: content-length:" ]]; then
                length="${line[2]}"
                length=$(echo "$length" | tr -d '\r')
                bytes=0
            elif [[ "$tag" == "<= recv" ]]; then
                local size="${line[3]}"
                bytes=$(( bytes + size ))
                if [[ "$length" -gt 0 ]]; then
                    print_progress "$bytes" "$length"
                fi
            fi
        done
    }

    wait "$curl_pid"
    local ret=$?
    echo "" >&4

    # Restore default EXIT trap now that curl is done
    trap - EXIT

    return "$ret"
}

# ── Resolve version ──────────────────────────────────────────────────────────

resolve_version() {
    if [[ -n "$requested_version" ]]; then
        # Strip leading 'v' if present
        requested_version="${requested_version#v}"

        # Verify the release exists
        local http_status
        http_status=$(curl -sI -o /dev/null -w "%{http_code}" \
            "https://github.com/prvious/pv/releases/tag/v${requested_version}")

        if [[ "$http_status" == "404" ]]; then
            echo -e "${RED}  Release v${requested_version} not found${NC}"
            echo -e "${MUTED}  Available releases: https://github.com/prvious/pv/releases${NC}"
            exit 1
        fi

        echo "$requested_version"
    else
        local version
        version=$(curl -s https://api.github.com/repos/prvious/pv/releases/latest \
            | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')

        if [[ -z "$version" ]]; then
            echo -e "${RED}  Failed to fetch latest version${NC}"
            echo -e "${MUTED}  Check your internet connection and try again.${NC}"
            echo -e "${MUTED}  Manual download: https://github.com/prvious/pv/releases${NC}"
            exit 1
        fi

        echo "$version"
    fi
}

# ── Check existing installation ──────────────────────────────────────────────

check_existing() {
    local existing
    existing=$(command -v pv 2>/dev/null || true)

    if [[ -n "$existing" ]]; then
        # Check if it's our pv (not the pipe viewer utility)
        if "$existing" --help 2>&1 | grep -q "FrankenPHP\|prvious\|pv install"; then
            local installed_version
            installed_version=$("$existing" version 2>/dev/null || echo "unknown")
            echo -e "${MUTED}  Existing installation detected: ${NC}$installed_version"
            echo -e "${MUTED}  Upgrading...${NC}"
        fi
    fi
}

# ── Shell PATH setup ─────────────────────────────────────────────────────────

add_to_path() {
    local config_file="$1"
    local command="$2"

    if grep -Fq "$command" "$config_file" 2>/dev/null; then
        return 0  # Already there
    fi

    if [[ -w "$config_file" ]]; then
        echo "" >> "$config_file"
        echo "# pv" >> "$config_file"
        echo "$command" >> "$config_file"
        echo -e "  ${GREEN}✓${NC} Added to PATH in ${MUTED}$config_file${NC}"
    else
        echo -e "  ${MUTED}Manually add to $config_file:${NC}"
        echo -e "    $command"
    fi
}

setup_path() {
    local current_shell
    current_shell=$(basename "${SHELL:-/bin/sh}")

    local config_file=""
    local eval_line=""

    case "$current_shell" in
        zsh)
            config_file="${ZDOTDIR:-$HOME}/.zshrc"
            eval_line='eval "$(pv env)"'
            ;;
        bash)
            # Prefer .bashrc, fall back to .bash_profile
            if [[ -f "$HOME/.bashrc" ]]; then
                config_file="$HOME/.bashrc"
            else
                config_file="$HOME/.bash_profile"
            fi
            eval_line='eval "$(pv env)"'
            ;;
        fish)
            config_file="$HOME/.config/fish/config.fish"
            eval_line='pv env | source'
            ;;
        *)
            config_file="$HOME/.profile"
            eval_line='eval "$(pv env)"'
            ;;
    esac

    if [[ ! -f "$config_file" ]]; then
        touch "$config_file"
    fi

    add_to_path "$config_file" "$eval_line"
}

# ── Header ───────────────────────────────────────────────────────────────────

print_header() {
    local version="$1"
    echo ""
    echo -e "  ${PURPLE}${BOLD}pv${NC} ${MUTED}v${version}${NC}"
    echo -e "  ${MUTED}Local PHP development, zero config${NC}"
    echo ""
}

# ── Outro ────────────────────────────────────────────────────────────────────

print_outro() {
    echo ""
    echo -e "  ${PURPLE}${BOLD}Ready!${NC} Get started:"
    echo ""
    echo -e "  ${BOLD}cd${NC} your-project  ${MUTED}# Go to a PHP project${NC}"
    echo -e "  ${BOLD}pv link${NC}           ${MUTED}# Link it as project.test${NC}"
    echo -e "  ${BOLD}pv start${NC}          ${MUTED}# Start the server${NC}"
    echo ""
    echo -e "  ${MUTED}Docs: https://github.com/prvious/pv${NC}"
    echo ""
}

# ── Main ─────────────────────────────────────────────────────────────────────

main() {
    # Detect platform
    local platform
    platform=$(detect_platform)

    # Resolve version
    local version
    version=$(resolve_version)

    # Print header
    print_header "$version"

    echo -e "  ${MUTED}Detected:${NC} macOS ${platform#darwin-}"
    echo ""

    # Resolve install directory: flag → default (~/.local/bin)
    local dest_dir="${install_dir:-$HOME/.local/bin}"
    mkdir -p "$dest_dir"

    # Acquire sudo credentials upfront if pv install needs it
    # (DNS resolver in /etc/resolver/ and CA trust require sudo).
    # Skip in CI where passwordless sudo is available.
    if [[ -z "${GITHUB_ACTIONS-}" ]]; then
        echo -e "  ${MUTED}pv needs sudo for DNS and certificate setup. You may be prompted for your password.${NC}"
        sudo -v
        echo ""
    fi

    # Check for existing installation
    check_existing

    # Download pv binary
    local url="https://github.com/prvious/pv/releases/download/v${version}/pv-${platform}"
    local tmp_dir="${TMPDIR:-/tmp}/pv_install_$$"
    mkdir -p "$tmp_dir"
    trap "rm -rf '$tmp_dir'" EXIT

    echo -e "  ${MUTED}Downloading pv...${NC}"

    if [[ -t 2 ]] && download_with_progress "$url" "$tmp_dir/pv" 2>&1; then
        : # progress bar showed
    else
        # Fallback: no TTY or progress bar failed
        if ! curl -fsSL -o "$tmp_dir/pv" "$url"; then
            echo ""
            echo -e "  ${RED}Failed to download pv${NC}"
            echo -e "  ${MUTED}Check your internet connection and try again.${NC}"
            echo -e "  ${MUTED}Manual download: https://github.com/prvious/pv/releases${NC}"
            echo ""
            exit 1
        fi
    fi

    chmod 755 "$tmp_dir/pv"

    # Install to destination directory
    echo -e "  ${MUTED}Installing to ${dest_dir}/pv...${NC}"
    mv "$tmp_dir/pv" "$dest_dir/pv"

    echo -e "  ${GREEN}✓${NC} pv v${version} installed"
    echo ""

    # Run pv install (full bootstrap — this calls sudo internally for DNS/CA)
    echo -e "  ${PURPLE}Setting up environment...${NC}"
    echo ""

    local install_args=(install --force)
    if [[ -n "$php_version" ]]; then
        install_args+=(--php "$php_version")
    fi
    if [[ -n "$tld" ]]; then
        install_args+=(--tld "$tld")
    fi

    "$dest_dir/pv" "${install_args[@]}"

    # Set up PATH
    echo ""
    if [[ "$no_modify_path" != "true" ]]; then
        setup_path
    fi

    # GitHub Actions support
    if [[ -n "${GITHUB_ACTIONS-}" ]] && [[ "${GITHUB_ACTIONS}" == "true" ]]; then
        echo "$dest_dir" >> "$GITHUB_PATH"
        echo "$HOME/.pv/bin" >> "$GITHUB_PATH"
        echo "$HOME/.pv/composer/vendor/bin" >> "$GITHUB_PATH"
        echo -e "  ${GREEN}✓${NC} Added to \$GITHUB_PATH"
    fi

    # Outro
    print_outro
}

main
