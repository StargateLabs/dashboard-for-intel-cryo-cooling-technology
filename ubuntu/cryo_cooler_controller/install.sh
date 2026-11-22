#!/usr/bin/env bash
# Build a local Debian package from the compiled Ubuntu binary.
set -euo pipefail

src=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(dirname -- "$src")
binary=$(realpath -- "${1:-$root/target/release/cryo_cooler_controller}")
if [[ ! -x "$binary" ]]; then
    printf 'Eseguibile assente: %s\n' "$binary" >&2
    exit 1
fi
arch=$(dpkg-architecture -qDEB_HOST_ARCH)
version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$src/Cargo.toml" | head -1)+emerald.11"
output=${2:-$root/dist/stargate-cryo_${version}_${arch}.deb}
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT

install -Dm755 "$binary" "$stage/usr/bin/stargate-cryo"
strip --strip-unneeded "$stage/usr/bin/stargate-cryo"
install -Dm644 "$src/stargate-cryo.desktop" "$stage/usr/share/applications/stargate-cryo.desktop"
install -Dm644 "$src/icon.png" "$stage/usr/share/icons/hicolor/64x64/apps/stargate-cryo.png"
install -Dm644 "$src/70-stargate-cryo.rules" "$stage/usr/lib/udev/rules.d/70-stargate-cryo.rules"
install -Dm644 "$root/LICENSE" "$stage/usr/share/doc/stargate-cryo/copyright"

cd "$stage"
mkdir -p DEBIAN
cat > DEBIAN/control <<CONTROL
Package: stargate-cryo
Version: $version
Architecture: $arch
Maintainer: Stargate Labs
Section: utils
Priority: optional
Depends: libc6 (>= 2.43), libgcc-s1, libudev1, libxdo3, libgtk-3-0t64, libgdk-pixbuf-2.0-0, libpango-1.0-0, libglib2.0-0t64, libayatana-appindicator3-1, fontconfig, libnotify-bin, fonts-liberation, udev
Recommends: libvulkan1, mesa-vulkan-drivers
Description: Stargate CryoCooling Dashboard Emerald for Ubuntu
 Native desktop monitoring and control for Intel CryoCooling TEC devices
 with Thermaltake Pacific TF2 and TF3 telemetry.
CONTROL
cat > DEBIAN/postinst <<'HOOK'
#!/bin/sh
set -e
if [ "$1" = configure ] && udevadm control --reload-rules 2>/dev/null; then
    udevadm trigger --action=change --subsystem-match=tty --property-match=ID_VENDOR_ID=10c4 --property-match=ID_MODEL_ID=ea60 || true
    udevadm trigger --action=change --subsystem-match=hidraw --property-match=ID_VENDOR_ID=264a --property-match=ID_MODEL_ID=233b || true
    udevadm trigger --action=change --subsystem-match=hidraw --property-match=ID_VENDOR_ID=264a --property-match=ID_MODEL_ID=232b || true
    udevadm trigger --action=change --subsystem-match=hidraw --property-match=ID_VENDOR_ID=264a --property-match=ID_MODEL_ID=232d || true
fi
HOOK
chmod 755 DEBIAN/postinst
cd "$root"
mkdir -p -- "$(dirname -- "$output")"
dpkg-deb -Zxz --root-owner-group --build "$stage" "$output"
