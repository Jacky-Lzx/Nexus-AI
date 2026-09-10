#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output_dir="$project_dir/target/swift"
module_cache="$output_dir/module-cache"
sdk_path=$(xcrun --sdk macosx --show-sdk-path)

case "$(uname -m)" in
    arm64) target="arm64-apple-macosx26.0"; interface_arch="arm64e" ;;
    x86_64) target="x86_64-apple-macosx26.0"; interface_arch="x86_64" ;;
    *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac

interface="$sdk_path/usr/lib/swift/Swift.swiftmodule/$interface_arch-apple-macos.swiftinterface"
sdk_swift_version=$(sed -n 's|// swift-compiler-version: Apple Swift version \([^ ]*\).*|\1|p' "$interface" | head -1)

mkdir -p "$output_dir" "$module_cache"
CLANG_MODULE_CACHE_PATH="$module_cache" swiftc \
    -emit-executable \
    -O \
    -swift-version 5 \
    -Xfrontend -interface-compiler-version \
    -Xfrontend "$sdk_swift_version" \
    -module-cache-path "$module_cache" \
    -sdk "$sdk_path" \
    -target "$target" \
    "$project_dir"/swift/Sources/ClipKeep/*.swift \
    -framework AppKit \
    -framework Carbon \
    -framework QuartzCore \
    -o "$output_dir/Nexus"

echo "Created $output_dir/Nexus"
