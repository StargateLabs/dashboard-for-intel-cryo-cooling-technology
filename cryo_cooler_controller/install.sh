#!/bin/bash
#
# Stargate Labs CryoCooling Dashboard — Linux Installer
# Run as: sudo ./install.sh
#

set -e

APP_NAME="stargate-cryo"
APP_DIR="/usr/local/bin"
DATA_DIR="$HOME/.local/share/$APP_NAME"
CONFIG_DIR="$HOME/.config/$APP_NAME"
DESKTOP_FILE="$HOME/.local/share/applications/$APP_NAME.desktop"
UDEV_RULE="/etc/udev/rules.d/99-$APP_NAME.rules"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Stargate Labs CryoCooling — Linux Installer${NC}"
echo "========================================"

# Check root (for udev rules)
if [ "$EUID" -eq 0 ]; then
    IS_ROOT=1
else
    IS_ROOT=0
    echo -e "${YELLOW}Warning: Not running as root.${NC}"
    echo "  - udev rules installation will be skipped"
    echo "  - Run with 'sudo' to install udev rules"
fi

# Find binary
BINARY=$(find . -name "$APP_NAME" -type f -executable 2>/dev/null | head -1)
if [ -z "$BINARY" ]; then
    # Try target/release
    if [ -f "target/release/$APP_NAME" ]; then
        BINARY="target/release/$APP_NAME"
    elif [ -f "target/debug/$APP_NAME" ]; then
        BINARY="target/debug/$APP_NAME"
    else
        echo -e "${RED}Error: Binary not found. Build first: cargo build --release${NC}"
        exit 1
    fi
fi

echo -e "Found binary: ${GREEN}$BINARY${NC}"

# Install binary
echo -n "Installing binary... "
cp "$BINARY" "$APP_DIR/$APP_NAME" && chmod +x "$APP_DIR/$APP_NAME"
echo -e "${GREEN}OK${NC}"

# Create data directory
echo -n "Creating data directory... "
mkdir -p "$DATA_DIR"
echo -e "${GREEN}OK${NC}"

# Create config directory
echo -n "Creating config directory... "
mkdir -p "$CONFIG_DIR"
echo -e "${GREEN}OK${NC}"

# Install desktop file
echo -n "Installing desktop file... "
mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Version=2.4
Name=Stargate CryoCooling
GenericName= TEC Controller
Comment=Stargate Labs CryoCooling Dashboard — Intel TEC Controller
Exec=$APP_DIR/$APP_NAME
Icon=stargate-cryo
Terminal=false
Type=Application
Categories=Utility;HardwareSettings;
Keywords=tec;cooling;cryo;temperature;
StartupNotify=true
EOF
chmod +x "$DESKTOP_FILE"
echo -e "${GREEN}OK${NC}"

# Install udev rules (as root)
if [ $IS_ROOT -eq 1 ]; then
    echo -n "Installing udev rules... "
    cat > "$UDEV_RULE" << 'EOF'
# Stargate Labs CryoCooling Dashboard
# USB-to-Serial adapters commonly used with Intel Cryo TEC controllers

# FTDI FT232R
SUBSYSTEM=="tty", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6001", MODE="0666", GROUP="dialout", SYMLINK+="stargate-cryo"

# CH340/CH341
SUBSYSTEM=="tty", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="7523", MODE="0666", GROUP="dialout", SYMLINK+="stargate-cryo"

# Silicon Labs CP2102/CP2103
SUBSYSTEM=="tty", ATTRS{idVendor}=="10c4", ATTRS{idProduct}=="ea60", MODE="0666", GROUP="dialout", SYMLINK+="stargate-cryo"

# Prolific PL2303
SUBSYSTEM=="tty", ATTRS{idVendor}=="067b", ATTRS{idProduct}=="2303", MODE="0666", GROUP="dialout", SYMLINK+="stargate-cryo"

# Default fallback for any ttyUSB/ttyACM (Cryo typically on these)
SUBSYSTEM=="tty", ATTRS{idVendor}=="0403", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="1a86", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="10c4", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="067b", MODE="0666", GROUP="dialout"
EOF
    chmod 644 "$UDEV_RULE"
    udevadm control --reload-rules
    echo -e "${GREEN}OK${NC}"
else
    echo -e "${YELLOW}Skipping udev rules (run with sudo)${NC}"
fi

# Summary
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "Installation complete!"
echo ""
echo "Paths:"
echo "  Binary:   $APP_DIR/$APP_NAME"
echo "  Config:  $CONFIG_DIR/config.json"
echo "  Data:    $DATA_DIR/sessions.db"
echo ""
echo "Run: $APP_DIR/$APP_NAME"
echo ""
echo "If using USB for the first time:"
echo "  1. Connect the CryoCooler"
echo "  2. List ports: ls /dev/ttyUSB* /dev/ttyACM*"
echo "  3. Run the app and select the port"
echo ""