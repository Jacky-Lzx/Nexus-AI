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

echo "Created $bundle_dir"
