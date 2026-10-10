#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-linux-x86_64}"
EXT="${2:-tar.xz}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

if [ -f "${ROOT_DIR}/versions.env" ]; then
    # shellcheck disable=SC1091
    source "${ROOT_DIR}/versions.env"
fi

PRESTASHOP_VERSION="${PRESTASHOP_VERSION:-9.0.3}"
STAGE_DIR="${ROOT_DIR}/build/stage/prestashop-portable-${TARGET}"
DIST_DIR="${ROOT_DIR}/dist"

echo "==> Packaging PrestaShop Portable bundle for ${TARGET} (${EXT})..."
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}" "${DIST_DIR}"

# 1. Launcher Binary
# NOTE: repo root is the Cargo workspace root, so `cargo build` (even when
# invoked from launcher/) outputs to <root>/target. Check that first.
# CI split-job handoff: $LAUNCHER_BIN (absolute or repo-relative) wins.
LAUNCHER_NAME="PrestaShopLauncher"
if [[ "${TARGET}" == windows* ]]; then
    LAUNCHER_NAME="PrestaShopLauncher.exe"
fi
SRC_BIN=""
if [ -n "${LAUNCHER_BIN:-}" ]; then
    case "${LAUNCHER_BIN}" in
        /*) SRC_BIN="${LAUNCHER_BIN}" ;;
        *) SRC_BIN="${ROOT_DIR}/${LAUNCHER_BIN}" ;;
    esac
fi
if [ -z "${SRC_BIN}" ] || [ ! -f "${SRC_BIN}" ]; then
    if [ -n "${LAUNCHER_BIN:-}" ]; then
        echo "Warning: LAUNCHER_BIN=${LAUNCHER_BIN} not found, falling back to target dirs"
    fi
    SRC_BIN=""
    if [[ "${TARGET}" == windows* ]]; then
        for candidate in \
            "${ROOT_DIR}/target/release/prestashop-launcher.exe" \
            "${ROOT_DIR}/launcher/target/release/prestashop-launcher.exe" \
            "${ROOT_DIR}/target/${TARGET}/release/prestashop-launcher.exe" \
            "${ROOT_DIR}/launcher/target/${TARGET}/release/prestashop-launcher.exe"
        do
            if [ -f "${candidate}" ]; then SRC_BIN="${candidate}"; break; fi
        done
    else
        for candidate in \
            "${ROOT_DIR}/target/release/prestashop-launcher" \
            "${ROOT_DIR}/launcher/target/release/prestashop-launcher" \
            "${ROOT_DIR}/target/${TARGET}/release/prestashop-launcher" \
            "${ROOT_DIR}/launcher/target/${TARGET}/release/prestashop-launcher"
        do
            if [ -f "${candidate}" ]; then SRC_BIN="${candidate}"; break; fi
        done
    fi
fi

if [ -n "${SRC_BIN}" ]; then
    cp "${SRC_BIN}" "${STAGE_DIR}/${LAUNCHER_NAME}"
    chmod +x "${STAGE_DIR}/${LAUNCHER_NAME}"
else
    echo "Warning: Launcher binary not found, creating placeholder"
    touch "${STAGE_DIR}/${LAUNCHER_NAME}"
fi

# 2. Config templates
mkdir -p "${STAGE_DIR}/config"
cp -r "${ROOT_DIR}/config/." "${STAGE_DIR}/config/"
rm -f "${STAGE_DIR}/config/nginx.conf" "${STAGE_DIR}/config/php.ini"

# 3. Runtime Binaries
mkdir -p "${STAGE_DIR}/runtime/${TARGET}"
if [ -d "${ROOT_DIR}/build/runtime/${TARGET}" ]; then
    cp -r "${ROOT_DIR}/build/runtime/${TARGET}/." "${STAGE_DIR}/runtime/${TARGET}/"
elif [ -d "${ROOT_DIR}/runtime/${TARGET}" ]; then
    cp -r "${ROOT_DIR}/runtime/${TARGET}/." "${STAGE_DIR}/runtime/${TARGET}/"
fi

# Ensure executable permissions on all runtime binaries
find "${STAGE_DIR}/runtime" -type f \( -name "*.sh" -o -name "php*" -o -name "mariadb*" -o -name "mysql*" -o -name "nginx*" -o -name "my_print_defaults" -o -name "resolveip" \) -exec chmod +x {} + 2>/dev/null || true

# On Windows, ensure VC++ runtime DLLs are placed in /runtime directories (php, mariadb/bin, nginx, and runtime root)
# so that the root folder remains clean and tidy without loose DLL files.
if [[ "${TARGET}" == windows* ]]; then
    VC_PATTERNS=("vcruntime*.dll" "msvcp*.dll" "vcomp*.dll" "concrt*.dll")
    PHP_RUNTIME_DIR="${STAGE_DIR}/runtime/${TARGET}/php"
    if [ -d "${PHP_RUNTIME_DIR}" ]; then
        for pattern in "${VC_PATTERNS[@]}"; do
            if [ -d "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin" ]; then
                find "${PHP_RUNTIME_DIR}" -maxdepth 1 -iname "${pattern}" -exec cp -f {} "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/" \; 2>/dev/null || true
            fi
            if [ -d "${STAGE_DIR}/runtime/${TARGET}/nginx" ]; then
                find "${PHP_RUNTIME_DIR}" -maxdepth 1 -iname "${pattern}" -exec cp -f {} "${STAGE_DIR}/runtime/${TARGET}/nginx/" \; 2>/dev/null || true
            fi
            find "${PHP_RUNTIME_DIR}" -maxdepth 1 -iname "${pattern}" -exec cp -f {} "${STAGE_DIR}/runtime/" \; 2>/dev/null || true
        done
    fi

    # Fallback: copy directly from Windows host System32 if any DLLs are missing in runtime dirs
    VC_DLLS=(vcruntime140.dll vcruntime140_1.dll msvcp140.dll msvcp140_1.dll msvcp140_2.dll msvcp140_codecvt_ids.dll vcomp140.dll concrt140.dll)
    for sys_dir in "/c/Windows/System32" "/c/Windows/SysWOW64" "C:/Windows/System32" "${WINDIR:-}/System32"; do
        if [ -n "${sys_dir}" ] && [ -d "${sys_dir}" ]; then
            for dll in "${VC_DLLS[@]}"; do
                found_dll=$(find "${sys_dir}" -maxdepth 1 -iname "${dll}" 2>/dev/null | head -n 1)
                if [ -n "${found_dll}" ] && [ -f "${found_dll}" ]; then
                    if [ -d "${STAGE_DIR}/runtime/${TARGET}/php" ] && [ ! -f "${STAGE_DIR}/runtime/${TARGET}/php/${dll}" ]; then
                        cp -f "${found_dll}" "${STAGE_DIR}/runtime/${TARGET}/php/${dll}"
                    fi
                    if [ -d "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin" ] && [ ! -f "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/${dll}" ]; then
                        cp -f "${found_dll}" "${STAGE_DIR}/runtime/${TARGET}/mariadb/bin/${dll}"
                    fi
                    if [ -d "${STAGE_DIR}/runtime/${TARGET}/nginx" ] && [ ! -f "${STAGE_DIR}/runtime/${TARGET}/nginx/${dll}" ]; then
                        cp -f "${found_dll}" "${STAGE_DIR}/runtime/${TARGET}/nginx/${dll}"
                    fi
                    if [ ! -f "${STAGE_DIR}/runtime/${dll}" ]; then
                        cp -f "${found_dll}" "${STAGE_DIR}/runtime/${dll}"
                    fi
                fi
            done
        fi
    done

    # Ensure root remains clean of any loose DLLs
    rm -f "${STAGE_DIR}"/*.dll
    echo "--> VC++ runtime DLLs placed in /runtime directories (root folder remains clean)"
fi

# 4. PrestaShop Core into app/
mkdir -p "${STAGE_DIR}/app"
if [ -d "${ROOT_DIR}/prestashop" ] && [ -f "${ROOT_DIR}/prestashop/autoload.php" ]; then
    echo "--> Copying PrestaShop from local prestashop/ directory..."
    cp -r "${ROOT_DIR}/prestashop/." "${STAGE_DIR}/app/"
elif [ -d "${ROOT_DIR}/app" ] && [ -f "${ROOT_DIR}/app/autoload.php" ]; then
    echo "--> Copying PrestaShop from local app/ directory..."
    cp -r "${ROOT_DIR}/app/." "${STAGE_DIR}/app/"
else
    echo "--> Downloading PrestaShop ${PRESTASHOP_VERSION} core archive..."
    # NOTE: PrestaShop 9.x GitHub releases ship no assets, so the core
    # archive is pinned as a release asset in this repo (see release
    # prestashop-core-<version>).
    PS_URL="https://github.com/VUXXE/prestashop-portable-rust/releases/download/prestashop-core-${PRESTASHOP_VERSION}/prestashop.zip"
    TEMP_PS_ZIP="$(mktemp --suffix=.zip 2>/dev/null || mktemp).zip"
    curl -fsSL -o "${TEMP_PS_ZIP}" "${PS_URL}"
    
    EXTRACT_TMP="$(mktemp -d)"
    unzip -q "${TEMP_PS_ZIP}" -d "${EXTRACT_TMP}"
    # PrestaShop zip often contains prestashop.zip inside it
    if [ -f "${EXTRACT_TMP}/prestashop.zip" ]; then
        unzip -q "${EXTRACT_TMP}/prestashop.zip" -d "${STAGE_DIR}/app"
    else
        cp -r "${EXTRACT_TMP}/." "${STAGE_DIR}/app/"
    fi
    rm -rf "${EXTRACT_TMP}" "${TEMP_PS_ZIP}"
fi

# 5. Data, Logs, Tmp empty skeleton
mkdir -p "${STAGE_DIR}/data/mariadb"
mkdir -p "${STAGE_DIR}/tmp/sessions"
mkdir -p "${STAGE_DIR}/tmp/uploads"
mkdir -p "${STAGE_DIR}/tmp/nginx_client_body"
mkdir -p "${STAGE_DIR}/tmp/nginx_proxy"
mkdir -p "${STAGE_DIR}/tmp/nginx_fastcgi"
mkdir -p "${STAGE_DIR}/tmp/nginx_uwsgi"
mkdir -p "${STAGE_DIR}/tmp/nginx_scgi"
mkdir -p "${STAGE_DIR}/temp"
mkdir -p "${STAGE_DIR}/logs"

# Touch .gitkeep
touch "${STAGE_DIR}/data/mariadb/.gitkeep"
touch "${STAGE_DIR}/tmp/sessions/.gitkeep"
touch "${STAGE_DIR}/tmp/uploads/.gitkeep"
touch "${STAGE_DIR}/tmp/nginx_client_body/.gitkeep"
touch "${STAGE_DIR}/tmp/nginx_proxy/.gitkeep"
touch "${STAGE_DIR}/tmp/nginx_fastcgi/.gitkeep"
touch "${STAGE_DIR}/tmp/nginx_uwsgi/.gitkeep"
touch "${STAGE_DIR}/tmp/nginx_scgi/.gitkeep"
touch "${STAGE_DIR}/temp/.gitkeep"
touch "${STAGE_DIR}/logs/.gitkeep"

# 6. Archive Creation
ARCHIVE_NAME="prestashop-portable-${TARGET}"
echo "--> Creating final archive: ${DIST_DIR}/${ARCHIVE_NAME}.${EXT}"

pushd "${ROOT_DIR}/build/stage" > /dev/null
case "${EXT}" in
    zip)
        if command -v zip >/dev/null 2>&1; then
            zip -r -q "${DIST_DIR}/${ARCHIVE_NAME}.zip" "${ARCHIVE_NAME}"
        elif command -v 7z >/dev/null 2>&1; then
            # Windows runners lack zip(1); 7-Zip is preinstalled there.
            7z a -tzip "${DIST_DIR}/${ARCHIVE_NAME}.zip" "${ARCHIVE_NAME}" > /dev/null
        else
            echo "Error: Neither zip nor 7z found" >&2
            exit 1
        fi
        ;;
    tar.xz)
        tar -cJf "${DIST_DIR}/${ARCHIVE_NAME}.tar.xz" "${ARCHIVE_NAME}"
        ;;
    tar.gz)
        tar -czf "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" "${ARCHIVE_NAME}"
        ;;
    *)
        echo "Error: Unknown extension ${EXT}" >&2
        exit 1
        ;;
esac
popd > /dev/null

echo "==> Package bundle created: ${DIST_DIR}/${ARCHIVE_NAME}.${EXT}"

# 7. Optional Windows Installer Creation (Inno Setup)
if [[ "${TARGET}" == windows* ]]; then
    ISCC_BIN=""
    for candidate in \
        "iscc" \
        "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
        "C:/Program Files (x86)/Inno Setup 6/ISCC.exe" \
        "/c/Program Files/Inno Setup 6/ISCC.exe" \
        "C:/Program Files/Inno Setup 6/ISCC.exe" \
        "/c/ProgramData/chocolatey/bin/iscc.exe"
    do
        if command -v "${candidate}" >/dev/null 2>&1 || [ -f "${candidate}" ]; then
            ISCC_BIN="${candidate}"
            break
        fi
    done

    if [ -n "${ISCC_BIN}" ] && [ -f "${ROOT_DIR}/ci/installer.iss" ]; then
        echo "--> Building Windows Inno Setup installer..."
        STAGE_DIR_WIN="$(cygpath -w "${STAGE_DIR}" 2>/dev/null || echo "${STAGE_DIR}")"
        DIST_DIR_WIN="$(cygpath -w "${DIST_DIR}" 2>/dev/null || echo "${DIST_DIR}")"
        ISS_FILE_WIN="$(cygpath -w "${ROOT_DIR}/ci/installer.iss" 2>/dev/null || echo "${ROOT_DIR}/ci/installer.iss")"
        ICON_FILE_WIN="$(cygpath -w "${ROOT_DIR}/launcher/icons/icon.ico" 2>/dev/null || echo "${ROOT_DIR}/launcher/icons/icon.ico")"

        MSYS2_ARG_CONV_EXCL="*" "${ISCC_BIN}" \
            "-dAppVersion=${LAUNCHER_VERSION:-1.0.3}" \
            "-dSourceDir=${STAGE_DIR_WIN}" \
            "-dOutputDir=${DIST_DIR_WIN}" \
            "-dOutputBaseFilename=${ARCHIVE_NAME}-installer" \
            "-dIconFile=${ICON_FILE_WIN}" \
            "${ISS_FILE_WIN}"
        echo "==> Windows installer created: ${DIST_DIR}/${ARCHIVE_NAME}-installer.exe"
    else
        echo "--> Inno Setup (iscc) not found or installer.iss missing, skipping .exe installer build."
    fi
fi
