#!/bin/sh
#
# obs-stats installer.
#
#   curl -fsSL https://raw.githubusercontent.com/worxbend/obs-stats/main/scripts/install.sh | sh
#
# Environment:
#   OBS_STATS_VERSION      tag to install (default: latest release)
#   OBS_STATS_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#
# POSIX sh: no bashisms, so it runs under dash, ash and busybox too.

set -eu

REPO="worxbend/obs-stats"
BIN="obs-stats"
INSTALL_DIR="${OBS_STATS_INSTALL_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

# --- work directory, cleaned up on every exit path -------------------------

TMP=""
cleanup() { [ -n "$TMP" ] && rm -rf "$TMP"; }
trap cleanup EXIT INT TERM

# --- platform detection ----------------------------------------------------

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os_part="unknown-linux-gnu" ;;
        Darwin) os_part="apple-darwin" ;;
        *)
            die "unsupported OS: $os (Windows users: download the .zip from the releases page)"
            ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch_part="x86_64" ;;
        aarch64|arm64) arch_part="aarch64" ;;
        *) die "unsupported architecture: $arch" ;;
    esac

    # macOS ships no aarch64-unknown-linux-gnu, and Linux has no
    # aarch64-apple-darwin problem — but musl systems need the static build.
    if [ "$os" = "Linux" ] && [ "$arch_part" = "x86_64" ] && ! ldd --version 2>&1 | grep -qi glibc; then
        os_part="unknown-linux-musl"
    fi

    printf '%s-%s' "$arch_part" "$os_part"
}

# --- version resolution ----------------------------------------------------

latest_version() {
    api="https://api.github.com/repos/$REPO/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        body="$(curl -fsSL "$api")" || die "could not reach the GitHub API"
    else
        body="$(wget -qO- "$api")" || die "could not reach the GitHub API"
    fi
    # Avoid a jq dependency: the tag is the first "tag_name" in the payload.
    printf '%s' "$body" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1
}

download() {
    url="$1"; dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$dest"
    else
        wget -qO "$dest" "$url"
    fi
}

# --- checksum verification -------------------------------------------------

verify() {
    archive="$1"; sums="$2"; name="$3"

    expected="$(grep " $name\$" "$sums" | awk '{print $1}' | head -n1)"
    [ -n "$expected" ] || die "no checksum published for $name"

    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$archive" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
    else
        say "warning: no sha256 tool found; skipping checksum verification"
        return 0
    fi

    [ "$actual" = "$expected" ] || die "checksum mismatch for $name
  expected $expected
  actual   $actual"

    info "checksum ok"
}

# --- main ------------------------------------------------------------------

command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 \
    || die "either curl or wget is required"
need tar
need uname

TARGET="$(detect_target)"
VERSION="${OBS_STATS_VERSION:-$(latest_version)}"
[ -n "$VERSION" ] || die "could not determine the latest version; set OBS_STATS_VERSION"

NAME="$BIN-$VERSION-$TARGET.tar.gz"
BASE="https://github.com/$REPO/releases/download/$VERSION"

say "Installing $BIN $VERSION"
info "target:  $TARGET"
info "archive: $NAME"

TMP="$(mktemp -d)"

download "$BASE/$NAME" "$TMP/$NAME" \
    || die "no release asset for $TARGET at $VERSION
  See https://github.com/$REPO/releases for what is available."

if download "$BASE/SHA256SUMS" "$TMP/SHA256SUMS" 2>/dev/null; then
    verify "$TMP/$NAME" "$TMP/SHA256SUMS" "$NAME"
else
    say "warning: SHA256SUMS not published for $VERSION; skipping verification"
fi

tar -xzf "$TMP/$NAME" -C "$TMP"

# The archive holds a single versioned directory; find the binary inside it.
SRC="$(find "$TMP" -type f -name "$BIN" -perm -u+x | head -n1)"
[ -n "$SRC" ] || die "the archive did not contain a $BIN binary"

mkdir -p "$INSTALL_DIR"
install -m 0755 "$SRC" "$INSTALL_DIR/$BIN" 2>/dev/null \
    || { cp "$SRC" "$INSTALL_DIR/$BIN" && chmod 0755 "$INSTALL_DIR/$BIN"; }

say ""
say "Installed $INSTALL_DIR/$BIN"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        say ""
        say "$INSTALL_DIR is not on your PATH. Add it:"
        say "  echo 'export PATH=\"\$PATH:$INSTALL_DIR\"' >> ~/.profile"
        ;;
esac

say ""
say "Next:"
say "  1. In OBS: Tools -> WebSocket Server Settings -> Enable WebSocket server"
say "  2. export OBS_WEBSOCKET_PASSWORD='...'   # from 'Show Connect Info'"
say "  3. $BIN"
