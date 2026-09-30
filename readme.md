# C-Road Editor

> A single-binary C editor that runs in your browser. No Python, no Node, no runtime — just one file and `gcc`.

[![Download](https://img.shields.io/badge/Download-Latest%20Release-blue?style=for-the-badge&logo=github)](https://github.com/Nour-yahyaoui/c-editor/releases/latest)

## Install

**Download the latest release:**
👉 **[github.com/Nour-yahyaoui/c-editor/releases/latest](https://github.com/Nour-yahyaoui/c-editor/releases/latest)**

Pick the file for your system:

| Platform | File | What to do |
|---|---|---|
| **Linux** | `c-road` | `chmod +x c-road && ./c-road` |
| **macOS** | `c-road` | `chmod +x c-road && ./c-road` |
| **Windows** | `c-road.exe` | Double-click it |

Then open **http://127.0.0.1:8080** in your browser.

You need `gcc` installed on the machine that runs the server:

- Ubuntu / Debian: `sudo apt install gcc`
- Fedora: `sudo dnf install gcc`
- Arch: `sudo pacman -S gcc`
- macOS: `xcode-select --install`
- Windows: install MinGW-w64 or use WSL

That's it. No Python, no Cargo, no config files.