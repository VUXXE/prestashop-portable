# PrestaShop Portable

[![Release Pipeline](https://github.com/VUXXE/prestashop-portable/actions/workflows/release.yml/badge.svg)](https://github.com/VUXXE/prestashop-portable/actions/workflows/release.yml)
[![Latest Release](https://img.shields.io/github/v/release/VUXXE/prestashop-portable?label=version)](https://github.com/VUXXE/prestashop-portable/releases/latest)
[![License: OSL-3.0 / MIT](https://img.shields.io/badge/license-OSL--3.0%20%2F%20MIT-blue.svg)](LICENSE)

A self-contained, high-performance **PrestaShop 9** distribution that runs locally without requiring installation, root/admin privileges, or system-wide dependencies on **Windows**, **Linux**, and **macOS** (Apple Silicon & Intel).

A lightweight native desktop launcher built with pure **Rust + GPUI** (GPU-accelerated UI framework) orchestrates all isolated runtime services (**Nginx 1.26**, **PHP 8.4 FastCGI / FPM**, and **MariaDB 11.4**).

---

## Download & Quick Start

1. **Download**: Grab the `.zip` archive for your platform from [Latest Releases](https://github.com/VUXXE/prestashop-portable/releases/latest) and extract it anywhere (e.g. Desktop, Documents, or external drive).
2. **Launch**: Run `PrestaShopLauncher` (or `PrestaShopLauncher.exe` on Windows).
3. **Start Services**: Click **Start Services** to boot MariaDB, PHP, and Nginx.
4. **Setup Shop**: Click **Start Shop Setup** to launch the browser wizard:
   - **Database Server**: `127.0.0.1` (Default Port: `3306`)
   - **Database Name**: `prestashop`
   - **Database Login**: `root`
   - **Database Password**: *(leave blank)*
5. **Access Back-Office**: Once the wizard completes, the launcher automatically detects your randomized admin URL and enables the **Admin Login** button.

---

## Features

- **100% Portable & Self-Contained**: Database (`data/`), web server configs (`config/`), temp uploads/sessions (`tmp/`), and logs (`logs/`) all live cleanly inside the portable bundle.
- **GPU-Accelerated Native GUI**: Powered by **Rust & GPUI** for instant startup, smooth 60+ FPS rendering, minimal memory usage, and native window controls.
- **Embedded Run-Time Dependencies**: All runtime binaries and shared libraries live cleanly inside `/runtime` — no external Visual C++ Redistributable or system libraries required on clean Windows installations.
- **Smart Admin Directory Detection**: Automatically detects the randomized admin directory name even after PrestaShop renames it for security.
- **One-Click Reinstall & Reset**: Built-in reset button to cleanly wipe the database, clear cache, restore the installer, and re-enable setup if an installation is interrupted.
- **Real-Time Log Stream**: Monitor Nginx access/error logs, PHP errors, and MariaDB logs directly from the desktop UI with filtering and auto-scroll.
- **Configurable Ports**: Easily change Web, PHP, and Database ports via the Settings panel to avoid port conflicts with existing local services.

---

## Directory Structure

```text
prestashop-portable/
├── PrestaShopLauncher       # Native launcher GUI (GPUI)
├── app/                     # PrestaShop 9 core application
├── config/                  # Nginx, PHP, and FastCGI templates
├── data/                    # MariaDB database data directory
├── logs/                    # Nginx, PHP, and MariaDB log files
├── runtime/                 # Isolated native binaries & shared libraries
├── temp/                    # PrestaShop installation temporary files
└── tmp/                     # Sessions, uploads, and FastCGI buffers
```

---

## Local Development

### Prerequisites

- **Rust toolchain** (stable 2021 edition)
- **Linux GUI dependencies** (only required when developing on Linux):
  ```bash
  sudo apt-get update
  sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libxcb-xkb-dev libfontconfig1-dev build-essential
  ```
  *(macOS and Windows only require the standard Rust toolchain).*

### Development Workflow

```bash
# 1. Setup local development stubs and directories
./scripts/setup-local-dev.sh

# 2. Run the launcher in development mode
cargo run -p prestashop-launcher

# 3. Code verification & tests
cargo test
```

Cross-platform multi-architecture release packages and runtime builds are fully automated via GitHub Actions.

---

## License

OSL-3.0 / MIT.
