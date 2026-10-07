#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ "$#" -eq 0 ]; then
    sh scripts/build-swift-plugin.sh plugins/ocr clipkeep-plugin-ocr
    binary="$project_dir/target/swift-plugins/clipkeep-plugin-ocr"
else
    binary="$1"
fi
sdk_path=$(xcrun --sdk macosx --show-sdk-path)
case "$(uname -m)" in
    arm64) interface_arch=arm64e ;;
    x86_64) interface_arch=x86_64 ;;
    *) echo "Unsupported macOS architecture" >&2; exit 1 ;;
esac
interface="$sdk_path/usr/lib/swift/Swift.swiftmodule/$interface_arch-apple-macos.swiftinterface"
sdk_swift_version=$(sed -n 's|// swift-compiler-version: Apple Swift version \([^ ]*\).*|\1|p' "$interface" | head -1)
module_cache="$project_dir/target/swift-plugins/module-cache"
mkdir -p "$module_cache"
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/nexus-ocr-tests.XXXXXX")
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

swiftc -swift-version 5 -Xfrontend -interface-compiler-version -Xfrontend "$sdk_swift_version" \
    -module-cache-path "$module_cache" -sdk "$sdk_path" \
    plugins/ocr/OCRPlugin.swift plugins/ocr/tests/workflow.swift -o "$test_dir/workflow"
"$test_dir/workflow"
swiftc -swift-version 5 -Xfrontend -interface-compiler-version -Xfrontend "$sdk_swift_version" \
    -module-cache-path "$module_cache" -sdk "$sdk_path" \
    plugins/ocr/tests/integration.swift -o "$test_dir/integration"
"$test_dir/integration" "$binary"
