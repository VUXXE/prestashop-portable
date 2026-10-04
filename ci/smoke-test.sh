#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-linux-x86_64}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

STAGE_DIR="${STAGE_DIR:-${ROOT_DIR}/build/stage/prestashop-portable-${TARGET}}"

echo "==> Running Smoke Tests for ${TARGET} in ${STAGE_DIR}..."

# Check directories
for dir in app config runtime data tmp logs; do
    if [ ! -d "${STAGE_DIR}/${dir}" ]; then
        echo "FAIL: Required directory missing: ${dir}" >&2
        exit 1
    fi
done

# Check essential configuration files
for cfg in nginx.conf.template php.ini.template mime.types fastcgi.conf; do
    if [ ! -f "${STAGE_DIR}/config/${cfg}" ]; then
        echo "FAIL: Required config file missing: ${cfg}" >&2
        exit 1
    fi
done

# Check launcher
LAUNCHER="PrestaShopLauncher"
[[ "${TARGET}" == windows* ]] && LAUNCHER="PrestaShopLauncher.exe"

if [ ! -f "${STAGE_DIR}/${LAUNCHER}" ]; then
    echo "FAIL: Launcher executable not found: ${STAGE_DIR}/${LAUNCHER}" >&2
    exit 1
fi

# Check PrestaShop index.php
if [ ! -f "${STAGE_DIR}/app/index.php" ]; then
    echo "FAIL: PrestaShop app/index.php not found!" >&2
    exit 1
fi

# Smoke test native binaries on the current runner
if [[ "$(uname -s)" == "Linux" && "${TARGET}" == "linux-x86_64" ]] || \
   [[ "$(uname -s)" == "Darwin" && "${TARGET}" == "macos"* ]]; then
    
    PHP_BIN="${STAGE_DIR}/runtime/${TARGET}/php/php-fpm"
    if [ -f "${PHP_BIN}" ]; then
        echo "--> Testing PHP binary: ${PHP_BIN} -v"
        [ -x "${PHP_BIN}" ] || { echo "FAIL: ${PHP_BIN} is not executable!" >&2; exit 1; }
        PHP_INI_SCAN_DIR="" "${PHP_BIN}" -v
    fi

    PHP_BIN="${STAGE_DIR}/runtime/${TARGET}/php/php-cgi"
    if [ -f "${PHP_BIN}" ]; then
        echo "--> Testing PHP binary: ${PHP_BIN} -v"
        [ -x "${PHP_BIN}" ] || { echo "FAIL: ${PHP_BIN} is not executable!" >&2; exit 1; }
        PHP_INI_SCAN_DIR="" "${PHP_BIN}" -v
    fi

    NGINX_BIN="${STAGE_DIR}/runtime/${TARGET}/nginx/sbin/nginx"
    if [ -f "${NGINX_BIN}" ]; then
        echo "--> Testing Nginx binary: ${NGINX_BIN} -v"
        [ -x "${NGINX_BIN}" ] || { echo "FAIL: ${NGINX_BIN} is not executable!" >&2; exit 1; }
        "${NGINX_BIN}" -v
    fi

    MARIADB_BIN="${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mariadbd"
    if [ -f "${MARIADB_BIN}" ]; then
        echo "--> Testing MariaDB binary: ${MARIADB_BIN} --version"
        [ -x "${MARIADB_BIN}" ] || { echo "FAIL: ${MARIADB_BIN} is not executable!" >&2; exit 1; }
        "${MARIADB_BIN}" --version
    fi

    # Check DB install script/binary
    INSTALL_DB=""
    if [ -f "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mariadb-install-db" ]; then
        INSTALL_DB="${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mariadb-install-db"
    elif [ -f "${STAGE_DIR}/runtime/${TARGET}/mariadb/scripts/mysql_install_db" ]; then
        INSTALL_DB="${STAGE_DIR}/runtime/${TARGET}/mariadb/scripts/mysql_install_db"
    fi
    if [ -n "${INSTALL_DB}" ]; then
        echo "--> Checking MariaDB installer: ${INSTALL_DB}"
        [ -x "${INSTALL_DB}" ] || { echo "FAIL: ${INSTALL_DB} is not executable!" >&2; exit 1; }
    fi

    if [[ "$(uname -s)" == "Darwin" ]]; then
        echo "--> Checking macOS binaries for non-portable Homebrew linkages..."
        for bin in "${STAGE_DIR}/runtime/${TARGET}/php/php-fpm" \
                   "${STAGE_DIR}/runtime/${TARGET}/nginx/sbin/nginx" \
                   "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mariadbd"; do
            if [ -f "${bin}" ]; then
                LEAKED=$(otool -L "${bin}" 2>/dev/null | grep -E "(/opt/homebrew|/usr/local)" || true)
                if [ -n "${LEAKED}" ]; then
                    echo "FAIL: ${bin} links directly to Homebrew path: ${LEAKED}" >&2
                    exit 1
                fi
            fi
        done
        echo "--> All macOS binaries are clean and portable (no leaked Homebrew paths)!"
    fi
fi

if [[ "${TARGET}" == windows* ]]; then
    PHP_BIN="${STAGE_DIR}/runtime/${TARGET}/php/php-cgi.exe"
    if [ -f "${PHP_BIN}" ]; then
        echo "--> Testing Windows PHP binary: ${PHP_BIN} -v"
        "${PHP_BIN}" -v

        EXT_DIR="${STAGE_DIR}/runtime/${TARGET}/php/ext"
        if [ -d "${EXT_DIR}" ]; then
            echo "--> Testing Windows PHP extensions (zip, pdo_mysql, curl, gd)..."
            "${PHP_BIN}" -d "extension_dir=${EXT_DIR}" -d "extension=zip" -d "extension=pdo_mysql" -d "extension=curl" -d "extension=gd" -m | grep -i "zip" || {
                echo "FAIL: PHP zip extension failed to load on Windows!" >&2
                exit 1
            }
            echo "--> Windows PHP zip extension verified!"
        fi
    fi

    NGINX_BIN="${STAGE_DIR}/runtime/${TARGET}/nginx/nginx.exe"
    if [ -f "${NGINX_BIN}" ]; then
        echo "--> Testing Windows Nginx binary: ${NGINX_BIN} -v"
        "${NGINX_BIN}" -v
    fi

    MARIADB_BIN="${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mariadbd.exe"
    [ -f "${MARIADB_BIN}" ] || MARIADB_BIN="${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/mysqld.exe"
    if [ -f "${MARIADB_BIN}" ]; then
        echo "--> Testing Windows MariaDB binary: ${MARIADB_BIN} --version"
        "${MARIADB_BIN}" --version
    fi

    # Verify launcher root is clean (no loose DLLs in root)
    if compgen -G "${STAGE_DIR}/*.dll" >/dev/null; then
        echo "FAIL: Loose DLLs found in launcher root (should be in runtime/):" >&2
        ls -la "${STAGE_DIR}"/*.dll >&2
        exit 1
    fi
    echo "--> Launcher root is clean (no loose DLLs)!"

    # Verify VC++ runtime DLLs are present in runtime directory
    for vc_dll in vcruntime140.dll msvcp140.dll; do
        if [ ! -f "${STAGE_DIR}/runtime/${TARGET}/php/${vc_dll}" ] && [ ! -f "${STAGE_DIR}/runtime/${vc_dll}" ]; then
            echo "FAIL: Required VC++ runtime DLL missing from runtime directory: ${vc_dll}" >&2
            exit 1
        fi
    done
    echo "--> VC++ runtime DLLs verified in /runtime!"
fi

echo "==> All Smoke Tests passed for ${TARGET}!"
