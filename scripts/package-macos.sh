#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bundle_dir="$project_dir/dist/Nexus.app"
legacy_bundle_dir="$project_dir/dist/ClipKeep.app"

if [ ! -e "$bundle_dir" ] && [ -d "$legacy_bundle_dir" ]; then
    mv "$legacy_bundle_dir" "$bundle_dir"
fi

cd "$project_dir"
MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --workspace --release
"$project_dir/scripts/build-swift.sh"
install -d "$bundle_dir/Contents/MacOS"
install -d "$bundle_dir/Contents/Resources"
install -d "$bundle_dir/Contents/PlugIns"
# Remove the executable name used by the former egui host when updating a bundle in place.
rm -f "$bundle_dir/Contents/MacOS/clipkeep" "$bundle_dir/Contents/MacOS/ClipKeep"
install -m 755 target/swift/Nexus "$bundle_dir/Contents/MacOS/Nexus"
applications_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.applications.clipkeepplugin"
caffeinate_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.caffeinate.clipkeepplugin"
clipboard_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.clipboard.clipkeepplugin"
calculator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.calculator.clipkeepplugin"
calendar_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.calendar.clipkeepplugin"
clash_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.clash.clipkeepplugin"
files_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.files.clipkeepplugin"
kill_process_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.kill-process.clipkeepplugin"
menu_bar_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.menu-bar.clipkeepplugin"
reminders_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.reminders.clipkeepplugin"
system_commands_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.system-commands.clipkeepplugin"
text_tools_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.text-tools.clipkeepplugin"
time_date_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.time-date.clipkeepplugin"
unit_converter_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.unit-converter.clipkeepplugin"
web_search_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.web-search.clipkeepplugin"
rm -f "$applications_plugin/clipkeep-plugin-applications" "$applications_plugin/plugin.toml"
rm -f "$clipboard_plugin/clipkeep-plugin-clipboard" "$clipboard_plugin/plugin.toml"
install -d "$applications_plugin/Contents/MacOS" "$applications_plugin/Contents/Resources"
install -d "$clipboard_plugin/Contents/MacOS" "$clipboard_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-applications "$applications_plugin/Contents/MacOS/clipkeep-plugin-applications"
install -m 644 plugins/applications/plugin.toml "$applications_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Applications-Info.plist "$applications_plugin/Contents/Info.plist"
install -d "$caffeinate_plugin/Contents/MacOS" "$caffeinate_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-caffeinate "$caffeinate_plugin/Contents/MacOS/clipkeep-plugin-caffeinate"
install -m 644 plugins/caffeinate/plugin.toml "$caffeinate_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Caffeinate-Info.plist "$caffeinate_plugin/Contents/Info.plist"
install -m 755 target/release/clipkeep-plugin-clipboard "$clipboard_plugin/Contents/MacOS/clipkeep-plugin-clipboard"
install -m 644 plugins/clipboard/plugin.toml "$clipboard_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Clipboard-Info.plist "$clipboard_plugin/Contents/Info.plist"
install -d "$calculator_plugin/Contents/MacOS" "$calculator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-calculator "$calculator_plugin/Contents/MacOS/clipkeep-plugin-calculator"
install -m 644 plugins/calculator/plugin.toml "$calculator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Calculator-Info.plist "$calculator_plugin/Contents/Info.plist"
install -d "$calendar_plugin/Contents/MacOS" "$calendar_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-calendar "$calendar_plugin/Contents/MacOS/clipkeep-plugin-calendar"
install -m 644 plugins/calendar/plugin.toml "$calendar_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Calendar-Info.plist "$calendar_plugin/Contents/Info.plist"
install -d "$clash_plugin/Contents/MacOS" "$clash_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-clash "$clash_plugin/Contents/MacOS/clipkeep-plugin-clash"
install -m 644 plugins/clash/plugin.toml "$clash_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Clash-Info.plist "$clash_plugin/Contents/Info.plist"
install -d "$files_plugin/Contents/MacOS" "$files_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-files "$files_plugin/Contents/MacOS/clipkeep-plugin-files"
install -m 644 plugins/files/plugin.toml "$files_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Files-Info.plist "$files_plugin/Contents/Info.plist"
install -d "$kill_process_plugin/Contents/MacOS" "$kill_process_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-kill-process "$kill_process_plugin/Contents/MacOS/clipkeep-plugin-kill-process"
install -m 644 plugins/kill-process/plugin.toml "$kill_process_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-KillProcess-Info.plist "$kill_process_plugin/Contents/Info.plist"
install -d "$menu_bar_plugin/Contents/MacOS" "$menu_bar_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-menu-bar "$menu_bar_plugin/Contents/MacOS/clipkeep-plugin-menu-bar"
install -m 644 plugins/menu-bar/plugin.toml "$menu_bar_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-MenuBar-Info.plist "$menu_bar_plugin/Contents/Info.plist"
install -d "$reminders_plugin/Contents/MacOS" "$reminders_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-reminders "$reminders_plugin/Contents/MacOS/clipkeep-plugin-reminders"
install -m 644 plugins/reminders/plugin.toml "$reminders_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Reminders-Info.plist "$reminders_plugin/Contents/Info.plist"
install -d "$system_commands_plugin/Contents/MacOS" "$system_commands_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-system-commands "$system_commands_plugin/Contents/MacOS/clipkeep-plugin-system-commands"
install -m 644 plugins/system-commands/plugin.toml "$system_commands_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-SystemCommands-Info.plist "$system_commands_plugin/Contents/Info.plist"
install -d "$text_tools_plugin/Contents/MacOS" "$text_tools_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-text-tools "$text_tools_plugin/Contents/MacOS/clipkeep-plugin-text-tools"
install -m 644 plugins/text-tools/plugin.toml "$text_tools_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-TextTools-Info.plist "$text_tools_plugin/Contents/Info.plist"
install -d "$time_date_plugin/Contents/MacOS" "$time_date_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-time-date "$time_date_plugin/Contents/MacOS/clipkeep-plugin-time-date"
install -m 644 plugins/time-date/plugin.toml "$time_date_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-TimeDate-Info.plist "$time_date_plugin/Contents/Info.plist"
install -d "$unit_converter_plugin/Contents/MacOS" "$unit_converter_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-unit-converter "$unit_converter_plugin/Contents/MacOS/clipkeep-plugin-unit-converter"
install -m 644 plugins/unit-converter/plugin.toml "$unit_converter_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-UnitConverter-Info.plist "$unit_converter_plugin/Contents/Info.plist"
install -d "$web_search_plugin/Contents/MacOS" "$web_search_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-web-search "$web_search_plugin/Contents/MacOS/clipkeep-plugin-web-search"
install -m 644 plugins/web-search/plugin.toml "$web_search_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-WebSearch-Info.plist "$web_search_plugin/Contents/Info.plist"
# Remove legacy plugin locations when updating a bundle in place.
rm -f "$bundle_dir/Contents/MacOS/clipkeep-plugin-applications"
rm -f "$bundle_dir/Contents/MacOS/clipkeep-plugin-clipboard"
install -m 644 packaging/macos/Info.plist "$bundle_dir/Contents/Info.plist"

python3 "$project_dir/scripts/package-plugin.py" "$project_dir/plugins/ocr" --output "$bundle_dir/Contents/PlugIns"

signing_identity=${CLIPKEEP_CODESIGN_IDENTITY:-}
if [ -z "$signing_identity" ]; then
    signing_identity=$(security find-identity -v -p codesigning 2>/dev/null \
        | awk -F '"' '/"ClipKeep Local Development"/ { print $2; exit }')
fi
if [ -n "$signing_identity" ]; then
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.applications \
        "$applications_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.caffeinate \
        "$caffeinate_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.clipboard \
        "$clipboard_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.calculator \
        "$calculator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.calendar \
        "$calendar_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.clash \
        "$clash_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.files \
        "$files_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.kill-process \
        "$kill_process_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.menu-bar \
        "$menu_bar_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.reminders \
        "$reminders_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.system-commands \
        "$system_commands_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.text-tools \
        "$text_tools_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.time-date \
        "$time_date_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.unit-converter \
        "$unit_converter_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.web-search \
        "$web_search_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        "$bundle_dir/Contents/PlugIns/io.clipkeep.ocr.clipkeepplugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep "$bundle_dir"
    codesign --verify --deep --strict "$bundle_dir"
    echo "Signed with $signing_identity"
else
    echo "warning: no stable signing identity found; Accessibility permission will reset after rebuilds" >&2
    echo "Create a development signing identity, or set CLIPKEEP_CODESIGN_IDENTITY." >&2
fi

echo "Created $bundle_dir"
