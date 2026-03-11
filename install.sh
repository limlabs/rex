#!/bin/sh
# Rex installer — downloads the latest release binary from GitHub.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/limlabs/rex/main/install.sh | sh
#
# Options (environment variables):
#   REX_VERSION   Install a specific version (e.g. "0.15.2"). Default: latest.
#   REX_INSTALL   Installation directory. Default: ~/.rex/bin

set -eu

REPO="limlabs/rex"
INSTALL_DIR="${REX_INSTALL:-$HOME/.rex/bin}"

# --- Helpers ---

info() { printf '  \033[1;35m%s\033[0m %s\n' "◆" "$1"; }
ok()   { printf '  \033[1;32m✓\033[0m \033[1;32m%s\033[0m\n' "$1"; }
err()  { printf '  \033[1;31m✗\033[0m %s\n' "$1" >&2; exit 1; }
dim()  { printf '  \033[2m%s\033[0m\n' "$1"; }
bold() { printf '  \033[1m%s\033[0m\n' "$1"; }

# --- Detect platform ---

detect_platform() {
  OS="$(uname -s)"
  ARCH="$(uname -m)"

  case "$OS" in
    Darwin) OS="macos" ;;
    Linux)  OS="linux" ;;
    *)      err "Unsupported OS: $OS" ;;
  esac

  case "$ARCH" in
    x86_64|amd64)  ARCH="x86_64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *)             err "Unsupported architecture: $ARCH" ;;
  esac

  PLATFORM="${OS}-${ARCH}"
}

# --- Resolve version ---

resolve_version() {
  if [ -n "${REX_VERSION:-}" ]; then
    VERSION="$REX_VERSION"
    return
  fi

  # Fetch latest release tag from GitHub API
  if command -v curl >/dev/null 2>&1; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/')"
  elif command -v wget >/dev/null 2>&1; then
    VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/')"
  else
    err "curl or wget is required"
  fi

  if [ -z "$VERSION" ]; then
    err "Could not determine latest version"
  fi
}

# --- Download and install ---

download() {
  TARBALL="rex-${PLATFORM}.tar.gz"
  URL="https://github.com/${REPO}/releases/download/v${VERSION}/${TARBALL}"

  TMPDIR="$(mktemp -d)"
  trap 'rm -rf "$TMPDIR"' EXIT

  info "Downloading rex v${VERSION} for ${PLATFORM}..."
  echo

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "${TMPDIR}/${TARBALL}" || err "Download failed. Check that v${VERSION} has a binary for ${PLATFORM}."
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$URL" -O "${TMPDIR}/${TARBALL}" || err "Download failed. Check that v${VERSION} has a binary for ${PLATFORM}."
  fi

  # Extract
  tar xzf "${TMPDIR}/${TARBALL}" -C "$TMPDIR"

  # Install
  mkdir -p "$INSTALL_DIR"
  mv "${TMPDIR}/rex" "${INSTALL_DIR}/rex"
  chmod +x "${INSTALL_DIR}/rex"
}

# --- Shell profile detection ---

add_to_path() {
  # Check if already on PATH
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*) return ;;
  esac

  SHELL_NAME="$(basename "${SHELL:-/bin/sh}")"
  case "$SHELL_NAME" in
    zsh)  PROFILE="$HOME/.zshrc" ;;
    bash)
      if [ -f "$HOME/.bashrc" ]; then
        PROFILE="$HOME/.bashrc"
      else
        PROFILE="$HOME/.bash_profile"
      fi
      ;;
    fish) PROFILE="$HOME/.config/fish/config.fish" ;;
    *)    PROFILE="$HOME/.profile" ;;
  esac

  LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
  if [ "$SHELL_NAME" = "fish" ]; then
    LINE="set -gx PATH ${INSTALL_DIR} \$PATH"
  fi

  if [ -f "$PROFILE" ] && grep -qF "$INSTALL_DIR" "$PROFILE" 2>/dev/null; then
    return
  fi

  echo >> "$PROFILE"
  echo "# Rex" >> "$PROFILE"
  echo "$LINE" >> "$PROFILE"

  ADDED_TO_PROFILE=1
}

# --- Main ---

main() {
  echo
  detect_platform
  resolve_version
  download

  ok "Rex v${VERSION} installed to ${INSTALL_DIR}/rex"
  echo

  ADDED_TO_PROFILE=0
  add_to_path

  if [ "$ADDED_TO_PROFILE" = "1" ]; then
    dim "Added ${INSTALL_DIR} to PATH in ${PROFILE}"
    dim "Restart your shell or run:"
    echo
    bold "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo
  fi

  dim "Get started:"
  echo
  bold "  rex init my-app"
  bold "  cd my-app"
  bold "  rex dev"
  echo
}

main
