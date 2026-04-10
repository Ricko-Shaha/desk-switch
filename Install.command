#!/bin/bash
set -e
cd "$(dirname "$0")"

echo ""
echo "╔══════════════════════════════════════════╗"
echo "║        Desk Switch — macOS Installer     ║"
echo "╚══════════════════════════════════════════╝"
echo ""

# ── 1. Check / Install Rust ──────────────────────────────────

if ! command -v cargo &>/dev/null; then
    echo "[1/6] Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo "       Rust installed."
else
    echo "[1/6] Rust found: $(cargo --version)"
fi

# ── 2. Check build dependencies (cmake, nasm for turbojpeg) ──

echo "[2/6] Checking build dependencies..."
if ! command -v cmake &>/dev/null; then
    echo "       Installing cmake..."
    if command -v brew &>/dev/null; then
        HOMEBREW_NO_AUTO_UPDATE=1 brew install cmake 2>/dev/null || \
        pip3 install --break-system-packages cmake 2>/dev/null || \
        pip3 install cmake 2>/dev/null
    else
        pip3 install --break-system-packages cmake 2>/dev/null || \
        pip3 install cmake 2>/dev/null
    fi
fi
if ! command -v nasm &>/dev/null; then
    echo "       Installing nasm..."
    if command -v brew &>/dev/null; then
        HOMEBREW_NO_AUTO_UPDATE=1 brew install nasm 2>/dev/null || true
    fi
fi
echo "       cmake: $(cmake --version 2>/dev/null | head -1 || echo 'not found')"
echo "       nasm:  $(nasm --version 2>/dev/null || echo 'not found (SIMD disabled)')"

# ── 3. Build ─────────────────────────────────────────────────

echo "[3/6] Building desk-switch (release)... this takes a few minutes the first time."
cargo build --release
echo "       Build complete."

# ── 3. Build virtual display helper ──────────────────────────

echo "[4/6] Building virtual display helper..."
if [ -f helpers/virtual-display-helper.m ]; then
    clang -framework Foundation -framework CoreGraphics \
        -o helpers/virtual-display-helper-bin \
        helpers/virtual-display-helper.m 2>&1 && \
        echo "       Helper compiled." || \
        echo "       Helper compilation failed (virtual display extend may not work)."
else
    echo "       Helper source not found (skipping)."
fi

# ── 4. Create .app bundle ────────────────────────────────────

APP_NAME="Desk Switch"
APP_DIR="$HOME/Applications/${APP_NAME}.app"

echo "[5/6] Creating ${APP_NAME}.app..."

mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

cp target/release/desk-switch "${APP_DIR}/Contents/MacOS/desk-switch"

# Copy virtual display helper into the bundle
if [ -f helpers/virtual-display-helper-bin ]; then
    cp helpers/virtual-display-helper-bin "${APP_DIR}/Contents/MacOS/virtual-display-helper"
    echo "       Virtual display helper included."
fi

# Ad-hoc sign the binary so macOS firewall accepts it
echo "       Code signing..."
codesign --force --sign - "${APP_DIR}/Contents/MacOS/desk-switch" 2>/dev/null && \
    echo "       Binary signed." || echo "       Signing skipped (non-critical)."

cat > "${APP_DIR}/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Desk Switch</string>
  <key>CFBundleDisplayName</key>
  <string>Desk Switch</string>
  <key>CFBundleIdentifier</key>
  <string>com.deskswitch.app</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleExecutable</key>
  <string>desk-switch</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>LSMinimumSystemVersion</key>
  <string>10.15</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>Desk Switch needs screen capture access to stream your display to the other machine.</string>
  <key>NSAppleEventsUsageDescription</key>
  <string>Desk Switch needs accessibility access to forward keyboard and mouse input.</string>
</dict>
</plist>
PLIST

# Generate app icon using Python + sips/iconutil
python3 - "${APP_DIR}/Contents/Resources" << 'PYICON'
import sys, os, struct, zlib

out_dir = sys.argv[1]

def make_png(size):
    pixels = []
    for y in range(size):
        row = []
        for x in range(size):
            cr = size * 0.22
            inside = True
            for ccx, ccy in [(cr, cr), (size-1-cr, cr), (cr, size-1-cr), (size-1-cr, size-1-cr)]:
                if (x < cr or x > size-1-cr) and (y < cr or y > size-1-cr):
                    if ((x-ccx)**2 + (y-ccy)**2) > cr**2:
                        inside = False
            if inside:
                t = y / size
                row.append((int(25 + t*35), int(35 + t*55), int(130 + t*100), 255))
            else:
                row.append((0, 0, 0, 0))
        pixels.append(row)

    font = {
        'D': [(0,0),(0,1),(0,2),(0,3),(0,4),(0,5),(0,6),(1,0),(2,0),(3,0),(3,1),
               (4,1),(4,2),(4,3),(4,4),(4,5),(3,5),(3,6),(2,6),(1,6)],
        'S': [(4,0),(3,0),(2,0),(1,0),(0,1),(0,2),(1,3),(2,3),(3,3),(4,4),(4,5),
               (3,6),(2,6),(1,6),(0,6)]
    }
    sc = max(1, size // 28)
    margin_x = int(size * 0.23)
    margin_y = int(size * 0.28)
    gap = int(size * 0.08)
    w_d = 5 * sc
    for ch, ox in [('D', margin_x), ('S', margin_x + w_d + gap)]:
        for px, py in font.get(ch, []):
            for dy in range(sc):
                for dx in range(sc):
                    fy = margin_y + py * sc + dy
                    fx = ox + px * sc + dx
                    if 0 <= fy < size and 0 <= fx < size:
                        pixels[fy][fx] = (255, 255, 255, 230)

    raw = b''
    for row in pixels:
        raw += b'\x00'
        for r, g, b, a in row:
            raw += struct.pack('BBBB', r, g, b, a)

    def chunk(ctype, data):
        c = ctype + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)

    hdr = struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', hdr) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')

# Write the master 512px PNG
master = os.path.join(out_dir, '_master.png')
with open(master, 'wb') as f:
    f.write(make_png(512))

print("       PNG generated, creating .icns...")
PYICON

MASTER_PNG="${APP_DIR}/Contents/Resources/_master.png"
ICONSET="${APP_DIR}/Contents/Resources/AppIcon.iconset"
if [ -f "$MASTER_PNG" ]; then
    mkdir -p "$ICONSET"
    sips -z 16  16  "$MASTER_PNG" --out "$ICONSET/icon_16x16.png"    >/dev/null 2>&1
    sips -z 32  32  "$MASTER_PNG" --out "$ICONSET/icon_16x16@2x.png" >/dev/null 2>&1
    sips -z 32  32  "$MASTER_PNG" --out "$ICONSET/icon_32x32.png"    >/dev/null 2>&1
    sips -z 64  64  "$MASTER_PNG" --out "$ICONSET/icon_32x32@2x.png" >/dev/null 2>&1
    sips -z 128 128 "$MASTER_PNG" --out "$ICONSET/icon_128x128.png"   >/dev/null 2>&1
    sips -z 256 256 "$MASTER_PNG" --out "$ICONSET/icon_128x128@2x.png" >/dev/null 2>&1
    sips -z 256 256 "$MASTER_PNG" --out "$ICONSET/icon_256x256.png"   >/dev/null 2>&1
    sips -z 512 512 "$MASTER_PNG" --out "$ICONSET/icon_256x256@2x.png" >/dev/null 2>&1
    cp "$MASTER_PNG"              "$ICONSET/icon_512x512.png"
    cp "$MASTER_PNG"              "$ICONSET/icon_512x512@2x.png"
    iconutil -c icns "$ICONSET" -o "${APP_DIR}/Contents/Resources/AppIcon.icns" 2>/dev/null && \
        echo "       Icon created." || echo "       iconutil failed, using generic icon."
    rm -rf "$ICONSET" "$MASTER_PNG"
fi

# Add icon reference to Info.plist if icns was created
if [ -f "${APP_DIR}/Contents/Resources/AppIcon.icns" ]; then
    sed -i '' 's|</dict>|  <key>CFBundleIconFile</key>\
  <string>AppIcon</string>\
</dict>|' "${APP_DIR}/Contents/Info.plist"
fi

echo "       App created at: ${APP_DIR}"

# ── 5. Launch ────────────────────────────────────────────────

echo "[6/6] Launching Desk Switch..."
echo ""
echo "╔══════════════════════════════════════════╗"
echo "║  Installation complete!                  ║"
echo "║                                          ║"
echo "║  The app is in: ~/Applications/          ║"
echo "║  You can also find it in Launchpad.      ║"
echo "║                                          ║"
echo "║  First time? The auth key is printed     ║"
echo "║  in the app — copy it to the other       ║"
echo "║  machine so they can connect.            ║"
echo "╚══════════════════════════════════════════╝"
echo ""

open "${APP_DIR}"
