#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bundle_dir="$project_dir/dist/ClipKeep.app"

cd "$project_dir"
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --workspace --release
"$project_dir/scripts/build-swift.sh"
install -d "$bundle_dir/Contents/MacOS"
install -d "$bundle_dir/Contents/Resources"
# Remove the executable name used by the former egui host when updating a bundle in place.
rm -f "$bundle_dir/Contents/MacOS/clipkeep"
install -m 755 target/swift/ClipKeep "$bundle_dir/Contents/MacOS/ClipKeep"
install -m 755 target/release/clipkeep-plugin-applications "$bundle_dir/Contents/MacOS/clipkeep-plugin-applications"
install -m 755 target/release/clipkeep-plugin-clipboard "$bundle_dir/Contents/MacOS/clipkeep-plugin-clipboard"
install -m 644 packaging/macos/Info.plist "$bundle_dir/Contents/Info.plist"

signing_identity=${CLIPKEEP_CODESIGN_IDENTITY:-}
if [ -z "$signing_identity" ]; then
    signing_identity=$(security find-identity -v -p codesigning 2>/dev/null \
        | awk -F '"' '/"ClipKeep Local Development"/ { print $2; exit }')
fi
if [ -n "$signing_identity" ]; then
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.applications \
        "$bundle_dir/Contents/MacOS/clipkeep-plugin-applications"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.clipboard \
        "$bundle_dir/Contents/MacOS/clipkeep-plugin-clipboard"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep "$bundle_dir"
    codesign --verify --deep --strict "$bundle_dir"
    echo "Signed with $signing_identity"
else
    echo "warning: no stable signing identity found; Accessibility permission will reset after rebuilds" >&2
    echo "Create a development signing identity, or set CLIPKEEP_CODESIGN_IDENTITY." >&2
fi

echo "Created $bundle_dir"
