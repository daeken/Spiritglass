#!/bin/bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
swift build --package-path "$project_dir/apps/macos"
binary_dir="$(swift build --package-path "$project_dir/apps/macos" --show-bin-path)"
bundle="$project_dir/.build/Spiritglass.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary_dir/Spiritglass" "$bundle/Contents/MacOS/Spiritglass"
cp "$project_dir/apps/macos/Info.plist" "$bundle/Contents/Info.plist"
iconset="$project_dir/.build/Spiritglass.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$project_dir/apps/macos/Resources/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina_size=$((size * 2))
    sips -z "$retina_size" "$retina_size" "$project_dir/apps/macos/Resources/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/Spiritglass.icns"
codesign --force --sign - "$bundle"
# Resource-only updates do not change the bundle directory's modification time.
# Refresh this app's metadata so Finder and the Dock see its current icon.
touch "$bundle"
registration_tool="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$registration_tool" ]]; then
    "$registration_tool" -f "$bundle"
fi
printf '%s\n' "$bundle"
