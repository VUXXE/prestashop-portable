#!/usr/bin/env bash
set -euo pipefail

APP_DIR="${1:-prestashop}"
TARGET="${2:-}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Find php binary
PHP_BIN=""
for candidate in \
    ${TARGET:+"${ROOT_DIR}/build/runtime/${TARGET}/php/php"} \
    ${TARGET:+"${ROOT_DIR}/build/runtime/${TARGET}/php/php.exe"} \
    ${TARGET:+"${APP_DIR}/../runtime/${TARGET}/php/php"} \
    ${TARGET:+"${APP_DIR}/../runtime/${TARGET}/php/php.exe"} \
    "${ROOT_DIR}/build/runtime/macos-arm64/php/php" \
    "${ROOT_DIR}/build/runtime/macos-x86_64/php/php" \
    "${ROOT_DIR}/build/runtime/linux-x86_64/php/php" \
    "${ROOT_DIR}/build/runtime/windows-x86_64/php/php.exe" \
    "${ROOT_DIR}/runtime/macos-arm64/php/php" \
    "${ROOT_DIR}/runtime/macos-x86_64/php/php" \
    "${ROOT_DIR}/runtime/linux-x86_64/php/php" \
    "${ROOT_DIR}/runtime/windows-x86_64/php/php.exe" \
    "php" \
    "/opt/homebrew/bin/php" \
    "/usr/local/bin/php"; do
    if [ -n "$candidate" ] && (command -v "$candidate" >/dev/null 2>&1 || [ -x "$candidate" ]); then
        PHP_BIN="$candidate"
        break
    fi
done

if [ -n "$PHP_BIN" ]; then
    "$PHP_BIN" "${SCRIPT_DIR}/patch-prestashop.php" "$APP_DIR"
    exit 0
fi

# Fallback: if php is not available on host (e.g. clean macOS runner), use Python 3
if command -v python3 >/dev/null 2>&1; then
    echo "--> Running patch-prestashop.py (fallback using Python 3)..."
    python3 "${SCRIPT_DIR}/patch-prestashop.py" "$APP_DIR"
    exit 0
fi

echo "Warning: Neither php nor python3 found to run patch-prestashop, skipping CI patch (launcher will patch on startup)"
