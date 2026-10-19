# Deimos Reader

**Deimos Reader** is an ultra-lightweight, fast, and memory-safe Open Source desktop PDF reader written in Rust (Edition 2021) for **Windows** and **Linux (X11 / Wayland)**.

Designed around the **Strict YAGNI** philosophy, it focuses exclusively on core PDF reading performance:
- **High-performance opening, navigation, and rendering** (virtual scrolling + zoom from `25%` to `400%`).
- **Fluid multi-tab interface** for viewing multiple documents simultaneously (with Drag & Drop support).
- **Native detection and decryption** of password-protected PDFs (`RC4`, `AES-128`, `AES-256`), automatic testing of the empty password `""`, and immediate wiping of cleartext passwords from RAM after authentication via [`zeroize`](https://crates.io/crates/zeroize).
- **Zero UI blocking & strict RAM budget**: asynchronous page rasterization in a dedicated background thread (`crossbeam-channel`) and a bounded GPU texture LRU cache limited to **15 pages maximum per tab** (`lru`).
- **Memory safety & minimalism**: `#![forbid(unsafe_code)]` across all application logic, no web engine (no Electron, no Tauri/WebView), and zero network calls or telemetry.

---

## Project Architecture

```text
├── Cargo.toml          # Dependencies and optimized Release profile (fat LTO, strip, panic=abort)
├── build.rs            # Windows PE resource script embedding the high-res Mars icon
├── README.md           # Documentation and build prerequisites
├── assets/
│   ├── Orbitron-Bold.ttf # Orbitron Bold (700) title font
│   ├── mars_logo.png     # High-resolution static Mars logo (256x256)
│   └── mars_icon.ico     # Multi-resolution Windows icon (16x16 to 256x256)
└── src/
    ├── main.rs         # Entry point, eframe configuration, icon & Windows console suppression
    ├── app.rs          # Global application state, tab bar, welcome view, keyboard shortcuts
    ├── tab.rs          # Tab state machine (Loading, Locked, Ready, Error)
    ├── pdf_engine.rs   # Isolated wrapper around the PDF engine (open, decrypt, RGBA8 render)
    ├── worker.rs       # Background worker thread managing per-tab rasterization
    └── cache.rs        # Bounded LRU cache of GPU textures (egui::TextureHandle, max 15 pages/tab)
```

---

## Keyboard Shortcuts & Controls

| Shortcut / Action | Description |
| :--- | :--- |
| `Ctrl + O` | Open one or more PDF files |
| `Ctrl + W` / Middle-click on tab | Close the active / clicked tab |
| `Ctrl + Tab` / `Ctrl + Shift + Tab` | Switch to the next / previous tab |
| `Ctrl + '+'` / `Ctrl + '-'` / `Ctrl + Mouse Wheel` | Zoom in / Zoom out (`25%` to `400%`) |
| `Ctrl + 0` | Reset zoom to `100%` |
| Drag & Drop | Open dropped `.pdf` files in new tabs |

---

## Build & Runtime Prerequisites

### 1. PDF Engine (`pdfium-render` / PDFium)

Deimos Reader interfaces with **PDFium** via `pdfium-render` (dynamically loading `pdfium.dll` on Windows or `libpdfium.so` on Linux) for full PDF specification compliance and native hardware-accelerated decryption (`RC4`, `AES-128`, `AES-256`).

The dynamic library is automatically searched in the following order:
1. The directory specified by the `PDFIUM_LIB_PATH` environment variable (if set).
2. The directory containing the `deimos-reader` executable (or its parent directories during development).
3. The current working directory (`.`).
4. Standard operating system library paths (`PATH` on Windows, `LD_LIBRARY_PATH` / `/usr/lib` / `/usr/local/lib` on Linux).

Prebuilt official PDFium binaries are available from [`bblanchon/pdfium-binaries`](https://github.com/bblanchon/pdfium-binaries/releases):
- **Windows (x64)**: place `bin/pdfium.dll` next to `deimos-reader.exe` (or at the repository root during development).
- **Linux (x64)**: place `lib/libpdfium.so` next to `deimos-reader` or in `/usr/local/lib/`.

---

### 2. Building on Windows (MSVC or MinGW)

#### Prerequisites
1. **Rust Toolchain** (`1.75+`, Edition 2021) installed via [rustup.rs](https://rustup.rs/):
   - **Recommended target (MSVC)**: `x86_64-pc-windows-msvc` with *Visual Studio C++ Build Tools* ("Desktop development with C++" and Windows 10/11 SDK).
   - **Alternative (MinGW)**: `x86_64-pc-windows-gnu` with MSYS2 / MinGW-w64 (`mingw-w64-x86_64-toolchain`).
2. **`pdfium.dll`** placed at the repository root (and bundled alongside `deimos-reader.exe` in the installer or release folder).

#### Commands
```powershell
# Static analysis and unit tests
cargo clippy --all-targets -- -D warnings
cargo test

# Optimized Release build (fat LTO, stripped symbols, no console window)
cargo build --release
```

---

### 3. Building on Linux (X11 / Wayland)

#### Prerequisites (System Packages)
On Linux, `eframe` / `winit` and `rfd` require standard graphics development headers (X11/Wayland/OpenGL) and GTK3 / XDG Desktop Portal for native file dialogs:

- **Debian / Ubuntu / Linux Mint**:
  ```bash
  sudo apt update
  sudo apt install -y build-essential pkg-config \
      libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
      libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
      libgl1-mesa-dev libegl1-mesa-dev libgtk-3-dev
  ```

- **Fedora / RHEL**:
  ```bash
  sudo dnf install -y gcc pkgconf-pkg-config \
      libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel \
      wayland-devel mesa-libGL-devel mesa-libEGL-devel gtk3-devel
  ```

- **Arch Linux**:
  ```bash
  sudo pacman -S --needed base-devel pkgconf libxcb libxkbcommon \
      libxkbcommon-x11 wayland mesa gtk3
  ```

#### Downloading `libpdfium.so` & Building
```bash
# Download libpdfium.so (x64) into the project root
curl -L https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz \
  | tar -xz lib/libpdfium.so --strip-components=1

# Lint, test, and build Release binary
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

---

## License

Distributed under the MIT Open Source License.
