#!/bin/sh
# build-ppa.sh — Prepare a Launchpad PPA source upload for superharness.
#
# Usage:
#   cd /path/to/superharness
#   packaging/ppa/build-ppa.sh [VERSION] [PPA_SERIES]
#
# Example:
#   packaging/ppa/build-ppa.sh 0.2.0 noble
#
# Requirements:
#   apt-get install devscripts debhelper dput gpg

set -e

VERSION="${1:-0.2.0}"
SERIES="${2:-noble}"
PKG="superharness"
FULLVER="${VERSION}-1"
BUILD_DIR="/tmp/ppa-build-${PKG}-${VERSION}"

# ── helpers ────────────────────────────────────────────────────────────────────
say() { printf '\033[1;32m==> \033[0m%s\n' "$*"; }
err() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "'$1' is required (install: apt-get install $2)"; }

need debuild  devscripts
need dput     dput
need gpg      gpg

# ── download upstream source tarball ──────────────────────────────────────────
say "Setting up build directory: ${BUILD_DIR}"
rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"

TARBALL="${PKG}_${VERSION}.orig.tar.gz"
UPSTREAM_URL="https://github.com/backmeupplz/superharness/archive/refs/tags/v${VERSION}.tar.gz"

say "Downloading upstream source: ${UPSTREAM_URL}"
curl -fSL -o "${BUILD_DIR}/${TARBALL}" "${UPSTREAM_URL}"

# ── extract and place debian/ directory ───────────────────────────────────────
say "Extracting source..."
cd "${BUILD_DIR}"
tar -xzf "${TARBALL}"
mv "${PKG}-${VERSION}" "${PKG}-${FULLVER}"

DEBIAN_SRC="$(dirname "$0")/../debian/debian"
if [ ! -d "${DEBIAN_SRC}" ]; then
    err "Could not find packaging/debian/debian/ — run this script from the repo root or adjust path."
fi

cp -r "${DEBIAN_SRC}" "${PKG}-${FULLVER}/debian"

# Update changelog to target the requested Ubuntu series
sed -i "s/) noble;/) ${SERIES};/" "${PKG}-${FULLVER}/debian/changelog"

# ── build the source package ───────────────────────────────────────────────────
say "Building source package (debuild -S -sa)..."
cd "${PKG}-${FULLVER}"
debuild -S -sa

# ── results ───────────────────────────────────────────────────────────────────
say "Source package built in: ${BUILD_DIR}"
ls -lh "${BUILD_DIR}"/*.dsc "${BUILD_DIR}"/*.tar.* 2>/dev/null || true

printf '\n'
say "To upload to Launchpad PPA, run:"
printf '  dput ppa:<your-launchpad-id>/superharness %s/%s_%s_source.changes\n' \
    "${BUILD_DIR}" "${PKG}" "${FULLVER}"
printf '\n'
say "First-time setup:"
printf '  1. Register at https://launchpad.net\n'
printf '  2. Create a PPA at https://launchpad.net/~<id>/+activate-ppa\n'
printf '  3. Upload your GPG key to the Ubuntu keyserver:\n'
printf '       gpg --keyserver keyserver.ubuntu.com --send-keys <YOUR_KEY_ID>\n'
printf '  4. Import it into Launchpad:\n'
printf '       https://launchpad.net/~<id>/+editpgpkeys\n'
printf '  5. Configure ~/.dput.cf (see packaging/ppa/README.md)\n'
