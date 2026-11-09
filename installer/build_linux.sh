#!/usr/bin/env bash
# Native Linux: portable archive, Ubuntu .deb, Arch/Arch Linux ARM package, symbols.
# See --help for --target, --target-dir, --dist-dir and --skip-build.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$here/package_unix.py" linux "$@"
