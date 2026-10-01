# Stargate Labs CryoCooling Dashboard — Linux Setup

## Supported Distributions
- **Ubuntu 22.04+** (primary)
- **Debian 12+** (compatible)
- **Fedora 38+** (compatible)

> **Note**: This app is designed for desktop use. Server/headless environments are not supported.

---

## Prerequisites

### 1. System Dependencies
```bash
# Ubuntu / Debian
sudo apt update
sudo apt install -y \
    libgtk-3-dev \
    libxkbcommon-dev \
    libwayland-dev \
    libudev-dev \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    liblm-sensors-dev

# Fedora
sudo dnf install -y \
    gtk3-devel \
    libxkbcommon-devel \
    libwayland-devel \
    libudev-devel \
    pkg-config \
    openssl-devel \
    sqlite-devel \
    lm_sensors-devel
```

### 2. Serial Port Access
The CryoCooler connects via USB-to-serial (USB-to-UART) chip.

```bash
# Add your user to the dialout group
sudo usermod -aG dialout $USER

# Log out and back in (or reboot) for group changes to take effect
```

### 3. lm-sensors (optional, for hardware monitoring)
```bash
# Install lm-sensors for CPU/GPU temperature reading
sudo apt install -y lm-sensors

# Detect hardware sensors
sudo sensors-detect
```

---

## Installation

### Option A: Manual Build
```bash
# Clone and build
git clone https://github.com/stargatelabs/stargate-cryo.git
cd stargate-cryo/stargate-cryo-fixed/cryo_cooler_controller
cargo build --release

# Run installation script
chmod +x ../install.sh
sudo ../install.sh
```

### Option B: Pre-built Binary
```bash
# Download release from GitHub Releases
# https://github.com/stargatelabs/stargate-cryo/releases

# Make executable
chmod +x stargate-cryo

# Run
./stargate-cryo
```

---

## USB Device Rules

The CryoCooler uses a USB-to-serial adapter. Common chipsets:
| Chipset | Vendor IDs |
|--------|-----------|
| FTDI   | `0403:6001` |
| CH340  | `1a86:7523` |
| CP2102 | `10c4:ea60` |
| PL2303 | `067b:2303` |

For automatic port detection (no need to specify `/dev/ttyUSB0`):
```bash
# List available ports
ls -la /dev/ttyUSB* /dev/ttyACM*
```

### Rule File (pre-installed by install.sh)
The rule file `99-stargate-cryo.rules` grants dialout group access to CryoCooler devices.

---

## Running the Application

### Desktop
Launch from application menu (after installation) or terminal:
```bash
stargate-cryo
```

### Command Line Options
```bash
stargate-cryo                    # Normal launch
WINIT_UNIX_BACKEND=x11 stargate-cryo  # Force X11 (if Wayland issues)
```

### Troubleshooting Display
- **X11 not responding**: Use `WINIT_UNIX_BACKEND=x11`
- **Wayland issues**: Set `WINIT_UNIX_BACKEND=wayland`
- **No display**: Ensure X11/Wayland server is running

---

## Data Locations

| Type | Path |
|------|------|
| Config | `~/.config/stargate-cryo/config.json` |
| Database | `~/.local/share/stargate-cryo/sessions.db` |
| Auto-save CSV | `~/.local/share/stargate-cryo/autosave.csv` |
| Logs | `~/.local/share/stargate-cryo/logs/` |
| PDF Reports | `~/Desktop/stargate_cryo_report_*.pdf` |
| CSV Export | `~/Desktop/stargate_cryo_*.csv` |

---

## Hardware Compatibility

### Tested USB-to-Serial Adapters
- FTDI FT232R
- CH340G/CH341
- Silicon Labs CP2102
- Prolific PL2303

### Not Compatible
- Bluetooth serial adapters (latency issues)
- Virtual COM ports

---

## Building from Source

### Install Rust
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### Build
```bash
cargo build --release --target x86_64-unknown-linux-gnu
```

---

## Uninstall
```bash
# Remove binary
sudo rm /usr/local/bin/stargate-cryo

# Remove desktop file
rm ~/.local/share/applications/stargate-cryo.desktop

# Remove udev rule
sudo rm /etc/udev/rules.d/99-stargate-cryo.rules
sudo udevadm control --reload-rules
```

---

## License
Stargate Labs CryoCooling Dashboard — Proprietary
Copyright (c) 2024 Stargate Labs