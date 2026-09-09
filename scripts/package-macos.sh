#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bundle_dir="$project_dir/dist/ClipKeep.app"

cd "$project_dir"
cargo build --release
install -d "$bundle_dir/Contents/MacOS"
install -d "$bundle_dir/Contents/Resources"
install -m 755 target/release/clipkeep "$bundle_dir/Contents/MacOS/clipkeep"
install -m 644 packaging/macos/Info.plist "$bundle_dir/Contents/Info.plist"

echo "Created $bundle_dir"
