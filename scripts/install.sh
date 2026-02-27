#!/usr/bin/env sh
# OpenSonarX CLI installer
# Usage: curl -fsSL https://opensonarx.com/install.sh | sh
set -e

REPO="bbiangul/opensonarx"
BINARY="oi"
INSTALL_DIR="${OI_INSTALL_DIR:-$HOME/.local/bin}"

# ── Detect platform ────────────────────────────────────────────────
detect_platform() {
  OS="$(uname -s)"
  ARCH="$(uname -m)"

  case "$OS" in
    Linux)  OS="linux" ;;
    Darwin) OS="darwin" ;;
    *)      echo "Error: unsupported OS: $OS"; exit 1 ;;
  esac

  case "$ARCH" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)   ARCH="aarch64" ;;
    *)               echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
  esac

  PLATFORM="${OS}-${ARCH}"
}

# ── Find latest release ────────────────────────────────────────────
get_latest_version() {
  if command -v curl >/dev/null 2>&1; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  elif command -v wget >/dev/null 2>&1; then
    VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  else
    echo "Error: curl or wget is required"
    exit 1
  fi

  if [ -z "$VERSION" ]; then
    echo "Error: could not determine latest version"
    exit 1
  fi
}

# ── Download and install ───────────────────────────────────────────
download_and_install() {
  TARBALL="oi-${PLATFORM}.tar.gz"
  URL="https://github.com/${REPO}/releases/download/${VERSION}/${TARBALL}"

  echo "  Platform:  ${PLATFORM}"
  echo "  Version:   ${VERSION}"
  echo "  Installing to: ${INSTALL_DIR}"
  echo ""

  TMPDIR="$(mktemp -d)"
  trap 'rm -rf "$TMPDIR"' EXIT

  echo "Downloading ${URL}..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "${TMPDIR}/${TARBALL}"
  else
    wget -q "$URL" -O "${TMPDIR}/${TARBALL}"
  fi

  tar -xzf "${TMPDIR}/${TARBALL}" -C "$TMPDIR"

  mkdir -p "$INSTALL_DIR"
  mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  chmod +x "${INSTALL_DIR}/${BINARY}"
}

# ── Verify ─────────────────────────────────────────────────────────
verify_install() {
  if [ -x "${INSTALL_DIR}/${BINARY}" ]; then
    echo ""
    echo "  OpenSonarX CLI installed successfully!"
    echo ""
    INSTALLED_VERSION="$("${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || echo "unknown")"
    echo "  ${INSTALLED_VERSION}"
    echo ""
  else
    echo "Error: installation failed"
    exit 1
  fi

  # Check if install dir is in PATH
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
      echo "  Add this to your shell profile to use 'oi' from anywhere:"
      echo ""
      echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
      echo ""
      ;;
  esac
}

# ── Main ───────────────────────────────────────────────────────────
main() {
  echo ""
  echo "  OpenSonarX CLI Installer"
  echo "  ========================"
  echo ""

  detect_platform
  get_latest_version
  download_and_install
  verify_install
}

main
