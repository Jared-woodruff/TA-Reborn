#!/usr/bin/env bash
# Native macOS: .app ZIP, drag-to-Applications DMG, dSYM symbols and checksums.
# Set MACOS_SIGN_IDENTITY for Developer ID signing; the default is ad-hoc signing.
# MACOS_NOTARY_PROFILE optionally names a configured notarytool keychain profile.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$here/package_unix.py" macos "$@"
