#!/bin/bash
# ms installer - https://github.com/quangdang46/ms
# Usage: curl -sSL https://raw.githubusercontent.com/quangdang46/ms/main/install.sh | bash
#
# Options:
#   --install-dir DIR  Install directory (default: ~/.local/bin)
#   --version VER      Version to install (default: latest)
#   --no-verify        Skip checksum verification
#   --easy-mode        Non-interactive, auto-configure PATH
#   --help             Show this help message
#
# Environment variables:
#   INSTALL_DIR        Override install directory
#   VERSION            Override version to install
#   VERIFY             Set to "false" to skip checksum verification
#   NO_COLOR           Disable colored output

set -euo pipefail

# Configuration
REPO="quangdang46/ms"
BINARY_NAME="ms"
DEFAULT_INSTALL_DIR="${HOME}/.local/bin"

# Colors (respect NO_COLOR)
if [[ -z "${NO_COLOR:-}" ]] && [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' NC=''
fi

log()  { echo -e "${BLUE}[ms]${NC} $*"; }
warn() { echo -e "${YELLOW}[ms]${NC} $*"; }
err()  { echo -e "${RED}[ms]${NC} $*" >&2; }
die()  { err "$*"; exit 1; }

usage() {
    cat << EOF
${BOLD}ms installer${NC}

Usage: $0 [OPTIONS]

Options:
  --install-dir DIR  Install directory (default: ~/.local/bin)
  --version VER      Version to install (default: latest)
  --no-verify        Skip checksum verification
  --easy-mode        Non-interactive, auto-configure PATH
  --help             Show this help message

Environment variables:
  INSTALL_DIR        Override install directory
  VERSION            Override version to install
  VERIFY             Set to "false" to skip checksum verification
  NO_COLOR           Disable colored output

Examples:
  # Install latest version
  curl -sSL https://raw.githubusercontent.com/quangdang46/ms/main/install.sh | bash

  # Non-interactive install with auto PATH configuration
  curl -sSL https://raw.githubusercontent.com/quangdang46/ms/main/install.sh | bash -s -- --easy-mode

  # Install specific version
  curl -sSL https://raw.githubusercontent.com/quangdang46/ms/main/install.sh | VERSION=v0.1.5 bash

  # Install to custom directory
  curl -sSL https://raw.githubusercontent.com/quangdang46/ms/main/install.sh | INSTALL_DIR=/usr/local/bin bash
EOF
}

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"

    case "$os" in
        linux)
            # Linux libc selection.
            #
            # The release pipeline historically published only the
            # `unknown-linux-gnu` artifact, which links against whatever
            # glibc the build runner provides. When that runner uses a newer
            # ubuntu image (e.g. glibc 2.39), users on Ubuntu 22.04 LTS
            # (glibc 2.35) get `GLIBC_2.38 not found` at run time.
            #
            # We previously hard-coded `unknown-linux-musl` for older glibc,
            # but the pipeline doesn't always publish a matching musl
            # artifact, so the installer 404'd. The robust approach is:
            #   1. Default to `unknown-linux-gnu`.
            #   2. Remember the host glibc so we can fall back to
            #      `unknown-linux-musl` if `gnu` 404s or the binary fails to
            #      run because of a glibc mismatch.
            os="unknown-linux-gnu"
            ;;
        darwin)
            os="apple-darwin"
            ;;
        mingw*|msys*|cygwin*)
            os="pc-windows-msvc"
            ;;
        *)
            die "Unsupported OS: $os"
            ;;
    esac

    case "$arch" in
        x86_64|amd64)
            arch="x86_64"
            ;;
        aarch64|arm64)
            arch="aarch64"
            ;;
        *)
            die "Unsupported architecture: $arch"
            ;;
    esac

    echo "${arch}-${os}"
}

is_release_version() {
    local version="${1#v}"
    [[ "$version" =~ ^[0-9]+[.][0-9]+[.][0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?([+][0-9A-Za-z][0-9A-Za-z.-]*)?$ ]]
}

normalize_version() {
    local version="$1"

    if [[ "$version" == "latest" ]]; then
        echo "latest"
        return 0
    fi

    if ! is_release_version "$version"; then
        die "Invalid version: $version (expected vX.Y.Z or X.Y.Z)"
    fi

    echo "v${version#v}"
}

require_option_value() {
    local option="$1"
    local value="${2:-}"

    if [[ -z "$value" || "$value" == --* ]]; then
        die "$option requires a value"
    fi
}

fetch_latest_version_from_redirect() {
    local effective_url version

    if command -v curl >/dev/null 2>&1; then
        effective_url=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest" 2>/dev/null) || return 1
    elif command -v wget >/dev/null 2>&1; then
        effective_url=$(
            wget -S --spider "https://github.com/${REPO}/releases/latest" 2>&1 |
                awk 'tolower($1) == "location:" { loc = $2 } END { sub(/\r$/, "", loc); print loc }'
        ) || return 1
        [[ -n "$effective_url" ]] || return 1
    else
        return 1
    fi
    version="${effective_url##*/}"

    if is_release_version "$version"; then
        normalize_version "$version"
        return 0
    fi

    return 1
}

# Fetch latest version without spending unauthenticated GitHub API quota.
fetch_latest_version() {
    local response version

    if version=$(fetch_latest_version_from_redirect); then
        echo "$version"
        return 0
    fi

    if command -v curl >/dev/null 2>&1; then
        response=$(curl -sS "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null) || {
            die "Failed to fetch latest version. Check your internet connection."
        }
    elif command -v wget >/dev/null 2>&1; then
        response=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null) || {
            die "Failed to fetch latest version. Check your internet connection."
        }
    else
        die "Neither curl nor wget found. Please install one of them."
    fi

    version=$(echo "$response" | grep -o '"tag_name": "[^"]*"' | head -1 | cut -d'"' -f4)

    if [[ -z "$version" ]]; then
        die "Could not determine latest version. The response was: $response"
    fi

    echo "$version"
}

# Download with progress
download() {
    local url="$1" dest="$2"
    local max_attempts="${MS_INSTALL_DOWNLOAD_ATTEMPTS:-3}"
    local attempt=1
    local status=0
    local delay=0

    log "Downloading from $url..."

    case "$max_attempts" in
        ''|*[!0-9]*) max_attempts=3 ;;
        0) max_attempts=1 ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        while [[ $attempt -le $max_attempts ]]; do
            if curl -fsSL "$url" -o "$dest"; then
                return 0
            else
                status=$?
            fi
            if [[ $attempt -lt $max_attempts ]]; then
                delay=$((attempt * 2))
                warn "Download failed (attempt ${attempt}/${max_attempts}); retrying in ${delay}s"
                sleep "$delay"
            fi
            attempt=$((attempt + 1))
        done
    elif command -v wget >/dev/null 2>&1; then
        while [[ $attempt -le $max_attempts ]]; do
            if wget -q "$url" -O "$dest"; then
                return 0
            else
                status=$?
            fi
            if [[ $attempt -lt $max_attempts ]]; then
                delay=$((attempt * 2))
                warn "Download failed (attempt ${attempt}/${max_attempts}); retrying in ${delay}s"
                sleep "$delay"
            fi
            attempt=$((attempt + 1))
        done
    else
        err "Neither curl nor wget found"
        return 1
    fi

    err "Download failed after ${max_attempts} attempt(s): $url"
    return "$status"
}

# Verify checksum
verify_checksum() {
    local artifact="$1" checksums="$2"
    local expected actual artifact_name

    if [[ "${VERIFY:-true}" != "true" ]]; then
        warn "Checksum verification skipped (--no-verify)"
        return 0
    fi

    if [[ ! -f "$checksums" ]]; then
        die "Checksums file not found: $checksums"
    fi

    artifact_name=$(basename "$artifact")
    expected=$(awk -v name="$artifact_name" '
        {
            filename = $NF
            sub(/\r$/, "", filename)
            if (filename == name) {
                print $1
                exit
            }
        }
    ' "$checksums")

    if [[ -z "$expected" ]]; then
        die "No checksum found for $artifact_name in $checksums"
    fi

    # Use sha256sum on Linux, shasum on macOS
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$artifact" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$artifact" | awk '{print $1}')
    else
        die "No SHA256 tool found. Install sha256sum or shasum, or rerun with --no-verify."
    fi

    if [[ "$expected" != "$actual" ]]; then
        die "Checksum mismatch! Expected: $expected, Got: $actual"
    fi

    log "Checksum verified ${GREEN}✓${NC}"
}

# Parse arguments
parse_args() {
    INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
    VERSION="${VERSION:-latest}"
    VERIFY="${VERIFY:-true}"
    EASY_MODE=0

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --install-dir)
                require_option_value "--install-dir" "${2:-}"
                INSTALL_DIR="$2"
                shift 2
                ;;
            --version)
                require_option_value "--version" "${2:-}"
                VERSION="$2"
                shift 2
                ;;
            --no-verify)
                VERIFY="false"
                shift
                ;;
            --easy-mode)
                EASY_MODE=1
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                die "Unknown option: $1. Use --help for usage."
                ;;
        esac
    done
}

# Main installation
main() {
    parse_args "$@"

    log "${BOLD}Installing ms...${NC}"

    # Detect platform
    local platform
    platform=$(detect_platform)
    log "Detected platform: ${GREEN}$platform${NC}"

    # Get version
    if [[ "$VERSION" == "latest" ]]; then
        log "Fetching latest version..."
        VERSION=$(fetch_latest_version)
    fi
    VERSION=$(normalize_version "$VERSION")
    log "Installing version: ${GREEN}$VERSION${NC}"

    # Create temp directory
    local temp_dir
    temp_dir=$(mktemp -d)
    trap 'rm -rf "${temp_dir:-}"' EXIT

    # Build download URLs
    # Adjust version for URL (strip 'v' prefix if present)
    local version_for_url="${VERSION#v}"
    local base_url="https://github.com/${REPO}/releases/download/${VERSION}"
    local archive_name="ms-${version_for_url}-${platform}.tar.gz"
    local archive_url="${base_url}/${archive_name}"
    local checksums_url="${base_url}/SHA256SUMS.txt"

    # Linux libc fallback: if the gnu artifact 404s OR the downloaded binary
    # fails to run (typically GLIBC version mismatch on older Linux), try the
    # statically-linked musl artifact before giving up. Older glibc hosts
    # (Ubuntu 22.04 LTS ships glibc 2.35) hit this when the release pipeline
    # builds the gnu artifact on a newer runner (glibc 2.38/2.39).
    install_artifact() {
        local archive_name="$1"
        local archive_url="$2"

        if [[ "$VERIFY" == "true" ]]; then
            download "$checksums_url" "${temp_dir}/SHA256SUMS.txt" || {
                err "Could not download checksums file: $checksums_url"
                return 1
            }
            verify_checksum "${temp_dir}/${archive_name}" "${temp_dir}/SHA256SUMS.txt" || return 1
        else
            warn "Checksum verification skipped (--no-verify)"
        fi

        log "Extracting..."
        local extract_dir="${temp_dir}/extract-${archive_name%.tar.gz}"
        rm -rf "$extract_dir"
        mkdir -p "$extract_dir"
        tar -xzf "${temp_dir}/${archive_name}" -C "$extract_dir" || {
            err "Failed to extract archive"
            return 1
        }

        local binary_path
        binary_path=$(find "$extract_dir" -name "$BINARY_NAME" -type f -executable 2>/dev/null | head -1)
        if [[ -z "$binary_path" ]]; then
            binary_path=$(find "$extract_dir" -name "$BINARY_NAME" -type f 2>/dev/null | head -1)
        fi
        if [[ -z "$binary_path" ]]; then
            err "Could not find $BINARY_NAME in archive"
            return 1
        fi

        mkdir -p "$INSTALL_DIR"
        mv -f "$binary_path" "${INSTALL_DIR}/${BINARY_NAME}"
        chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
        return 0
    }

    # Capture the original gnu artifact info up front so we can fall back to
    # musl whether the download 404s or the resulting binary fails to run.
    local primary_archive_name="$archive_name"
    local primary_archive_url="$archive_url"
    local primary_platform="$platform"
    local using_fallback=0

    local primary_failed=0
    if ! download "$primary_archive_url" "${temp_dir}/${primary_archive_name}"; then
        primary_failed=1
    fi

    if [[ $primary_failed -eq 1 ]] && [[ "$primary_platform" == *unknown-linux-gnu ]]; then
        warn "gnu artifact unavailable; trying statically-linked musl build..."
        platform="${primary_platform%-unknown-linux-gnu}-unknown-linux-musl"
        archive_name="ms-${version_for_url}-${platform}.tar.gz"
        archive_url="${base_url}/${archive_name}"
        if download "$archive_url" "${temp_dir}/${archive_name}"; then
            primary_failed=0
            using_fallback=1
        fi
    fi

    if [[ $primary_failed -eq 1 ]]; then
        die "Download failed: $primary_archive_url"
    fi

    install_artifact "$archive_name" "$archive_url" || die "Installation failed"

    log "${GREEN}${BOLD}Successfully installed ms ${VERSION} to ${INSTALL_DIR}/${BINARY_NAME}${NC}"

    # Detect GLIBC-mismatch failures on linux-gnu *after* install: the
    # download succeeded but the binary refuses to run because it was built
    # against a newer glibc than the host provides. In that case, transparently
    # re-download the musl artifact (statically linked, no glibc dependency)
    # and reinstall over the broken binary. This is the case Ubuntu 22.04 LTS
    # users hit and previously had to fix manually.
    if [[ $using_fallback -eq 0 ]] && [[ "$primary_platform" == *unknown-linux-gnu ]]; then
        local verify_stderr
        verify_stderr=$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>&1 1>/dev/null) || verify_stderr="$verify_stderr"
        if ! "${INSTALL_DIR}/${BINARY_NAME}" --version >/dev/null 2>&1; then
            warn "Installed binary failed to run; likely GLIBC mismatch."
            warn "Falling back to statically-linked musl build automatically..."
            if [[ -n "$verify_stderr" ]]; then
                warn "  (host reported: ${verify_stderr%%$'\n'*})"
            fi
            platform="${primary_platform%-unknown-linux-gnu}-unknown-linux-musl"
            archive_name="ms-${version_for_url}-${platform}.tar.gz"
            archive_url="${base_url}/${archive_name}"
            if download "$archive_url" "${temp_dir}/${archive_name}"; then
                if install_artifact "$archive_name" "$archive_url"; then
                    using_fallback=1
                    log "Reinstalled with musl artifact: ${GREEN}${archive_name}${NC}"
                else
                    err "Could not reinstall with musl artifact"
                fi
            else
                err "Could not download musl fallback artifact: $archive_url"
            fi
        fi
    fi

    # Check PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -q "^${INSTALL_DIR}$"; then
        if [[ "$EASY_MODE" -eq 1 ]]; then
            # Auto-add to PATH in easy mode (matches ACFS installer convention)
            local updated=0
            for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
                if [[ -e "$rc" ]] && [[ -w "$rc" ]]; then
                    if ! grep -F "$INSTALL_DIR" "$rc" >/dev/null 2>&1; then
                        echo "export PATH=\"\$PATH:${INSTALL_DIR}\"" >> "$rc"
                    fi
                    updated=1
                fi
            done
            if [[ "$updated" -eq 1 ]]; then
                warn "PATH updated in shell rc files; restart shell to use ms"
            else
                warn "Add ${INSTALL_DIR} to PATH to use ms"
            fi
        else
            echo ""
            warn "Add ${INSTALL_DIR} to your PATH:"
            echo ""
            echo "  For bash (add to ~/.bashrc):"
            echo "    export PATH=\"\$PATH:${INSTALL_DIR}\""
            echo ""
            echo "  For zsh (add to ~/.zshrc):"
            echo "    export PATH=\"\$PATH:${INSTALL_DIR}\""
            echo ""
            echo "  For fish (run once):"
            echo "    fish_add_path ${INSTALL_DIR}"
            echo ""
        fi
    fi

    # Run version check
    echo ""
    log "Verifying installation..."
    if "${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null; then
        echo ""
        log "${GREEN}Installation complete! Run 'ms --help' to get started.${NC}"
    else
        warn "Binary installed but failed to run."
        warn "This is often caused by a GLIBC version mismatch on older Linux"
        warn "distributions (e.g. Ubuntu 22.04 LTS ships glibc 2.35)."
        warn ""
        warn "Workarounds, in order of preference:"
        warn "  1. Re-run this installer; it will try the statically-linked"
        warn "     musl artifact if the gnu artifact is incompatible."
        warn "  2. Build from source with a recent toolchain:"
        warn "       rustup update stable   # need >= 1.85"
        warn "       cargo install --git https://github.com/${REPO}"
        warn "  3. Install via the project's Homebrew tap or Scoop bucket."
        exit 1
    fi
}

if [[ "${1:-}" == "--source-only" ]]; then
    if (return 0 2>/dev/null); then
        return 0
    fi
    exit 0
fi

main "$@"
