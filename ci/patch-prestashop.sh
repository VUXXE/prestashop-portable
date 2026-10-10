#!/usr/bin/env bash
set -euo pipefail

APP_DIR="${1:-prestashop}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Find php binary
PHP_BIN=""
for candidate in \
    "${ROOT_DIR}/runtime/linux-x86_64/php/php" \
    "${ROOT_DIR}/runtime/windows-x86_64/php/php.exe" \
    "${ROOT_DIR}/runtime/macos-arm64/php/php" \
    "${ROOT_DIR}/runtime/macos-x86_64/php/php" \
    "php"; do
    if command -v "$candidate" >/dev/null 2>&1 || [ -x "$candidate" ]; then
        PHP_BIN="$candidate"
        break
    fi
done

if [ -z "$PHP_BIN" ]; then
    echo "Warning: php binary not found to run patch-prestashop.php, skipping CI patch (launcher will patch on startup)"
    exit 0
fi

"$PHP_BIN" "${SCRIPT_DIR}/patch-prestashop.php" "$APP_DIR"
