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
base_converter_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.base-converter.clipkeepplugin"
caffeinate_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.caffeinate.clipkeepplugin"
clipboard_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.clipboard.clipkeepplugin"
color_tools_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.color-tools.clipkeepplugin"
cron_schedule_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.cron-schedule.clipkeepplugin"
csv_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.csv-inspector.clipkeepplugin"
date_calculator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.date-calculator.clipkeepplugin"
dns_lookup_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.dns-lookup.clipkeepplugin"
encoding_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.encoding-inspector.clipkeepplugin"
file_checksum_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.file-checksum.clipkeepplugin"
file_info_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.file-info.clipkeepplugin"
calculator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.calculator.clipkeepplugin"
calendar_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.calendar.clipkeepplugin"
clash_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.clash.clipkeepplugin"
files_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.files.clipkeepplugin"
http_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.http-inspector.clipkeepplugin"
id_generator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.id-generator.clipkeepplugin"
id_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.id-inspector.clipkeepplugin"
json_query_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.json-query.clipkeepplugin"
jwt_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.jwt-inspector.clipkeepplugin"
kill_process_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.kill-process.clipkeepplugin"
menu_bar_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.menu-bar.clipkeepplugin"
network_calculator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.network-calculator.clipkeepplugin"
password_generator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.password-generator.clipkeepplugin"
permission_calculator_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.permission-calculator.clipkeepplugin"
port_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.port-inspector.clipkeepplugin"
quick_notes_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.quick-notes.clipkeepplugin"
randomizer_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.randomizer.clipkeepplugin"
regex_tester_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.regex-tester.clipkeepplugin"
reminders_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.reminders.clipkeepplugin"
system_commands_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.system-commands.clipkeepplugin"
system_status_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.system-status.clipkeepplugin"
text_diff_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.text-diff.clipkeepplugin"
text_tools_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.text-tools.clipkeepplugin"
time_date_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.time-date.clipkeepplugin"
unicode_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.unicode-inspector.clipkeepplugin"
unit_converter_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.unit-converter.clipkeepplugin"
url_inspector_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.url-inspector.clipkeepplugin"
web_search_plugin="$bundle_dir/Contents/PlugIns/io.clipkeep.web-search.clipkeepplugin"
rm -f "$applications_plugin/clipkeep-plugin-applications" "$applications_plugin/plugin.toml"
rm -f "$clipboard_plugin/clipkeep-plugin-clipboard" "$clipboard_plugin/plugin.toml"
install -d "$applications_plugin/Contents/MacOS" "$applications_plugin/Contents/Resources"
install -d "$clipboard_plugin/Contents/MacOS" "$clipboard_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-applications "$applications_plugin/Contents/MacOS/clipkeep-plugin-applications"
install -m 644 plugins/applications/plugin.toml "$applications_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Applications-Info.plist "$applications_plugin/Contents/Info.plist"
install -d "$base_converter_plugin/Contents/MacOS" "$base_converter_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-base-converter "$base_converter_plugin/Contents/MacOS/clipkeep-plugin-base-converter"
install -m 644 plugins/base-converter/plugin.toml "$base_converter_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-BaseConverter-Info.plist "$base_converter_plugin/Contents/Info.plist"
install -d "$caffeinate_plugin/Contents/MacOS" "$caffeinate_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-caffeinate "$caffeinate_plugin/Contents/MacOS/clipkeep-plugin-caffeinate"
install -m 644 plugins/caffeinate/plugin.toml "$caffeinate_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Caffeinate-Info.plist "$caffeinate_plugin/Contents/Info.plist"
install -m 755 target/release/clipkeep-plugin-clipboard "$clipboard_plugin/Contents/MacOS/clipkeep-plugin-clipboard"
install -m 644 plugins/clipboard/plugin.toml "$clipboard_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Clipboard-Info.plist "$clipboard_plugin/Contents/Info.plist"
install -d "$color_tools_plugin/Contents/MacOS" "$color_tools_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-color-tools "$color_tools_plugin/Contents/MacOS/clipkeep-plugin-color-tools"
install -m 644 plugins/color-tools/plugin.toml "$color_tools_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-ColorTools-Info.plist "$color_tools_plugin/Contents/Info.plist"
install -d "$cron_schedule_plugin/Contents/MacOS" "$cron_schedule_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-cron-schedule "$cron_schedule_plugin/Contents/MacOS/clipkeep-plugin-cron-schedule"
install -m 644 plugins/cron-schedule/plugin.toml "$cron_schedule_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-CronSchedule-Info.plist "$cron_schedule_plugin/Contents/Info.plist"
install -d "$csv_inspector_plugin/Contents/MacOS" "$csv_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-csv-inspector "$csv_inspector_plugin/Contents/MacOS/clipkeep-plugin-csv-inspector"
install -m 644 plugins/csv-inspector/plugin.toml "$csv_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-CSVInspector-Info.plist "$csv_inspector_plugin/Contents/Info.plist"
install -d "$date_calculator_plugin/Contents/MacOS" "$date_calculator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-date-calculator "$date_calculator_plugin/Contents/MacOS/clipkeep-plugin-date-calculator"
install -m 644 plugins/date-calculator/plugin.toml "$date_calculator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-DateCalculator-Info.plist "$date_calculator_plugin/Contents/Info.plist"
install -d "$dns_lookup_plugin/Contents/MacOS" "$dns_lookup_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-dns-lookup "$dns_lookup_plugin/Contents/MacOS/clipkeep-plugin-dns-lookup"
install -m 644 plugins/dns-lookup/plugin.toml "$dns_lookup_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-DNSLookup-Info.plist "$dns_lookup_plugin/Contents/Info.plist"
install -d "$encoding_inspector_plugin/Contents/MacOS" "$encoding_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-encoding-inspector "$encoding_inspector_plugin/Contents/MacOS/clipkeep-plugin-encoding-inspector"
install -m 644 plugins/encoding-inspector/plugin.toml "$encoding_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-EncodingInspector-Info.plist "$encoding_inspector_plugin/Contents/Info.plist"
install -d "$file_checksum_plugin/Contents/MacOS" "$file_checksum_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-file-checksum "$file_checksum_plugin/Contents/MacOS/clipkeep-plugin-file-checksum"
install -m 644 plugins/file-checksum/plugin.toml "$file_checksum_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-FileChecksum-Info.plist "$file_checksum_plugin/Contents/Info.plist"
install -d "$file_info_plugin/Contents/MacOS" "$file_info_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-file-info "$file_info_plugin/Contents/MacOS/clipkeep-plugin-file-info"
install -m 644 plugins/file-info/plugin.toml "$file_info_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-FileInfo-Info.plist "$file_info_plugin/Contents/Info.plist"
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
install -d "$http_inspector_plugin/Contents/MacOS" "$http_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-http-inspector "$http_inspector_plugin/Contents/MacOS/clipkeep-plugin-http-inspector"
install -m 644 plugins/http-inspector/plugin.toml "$http_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-HTTPInspector-Info.plist "$http_inspector_plugin/Contents/Info.plist"
install -d "$id_generator_plugin/Contents/MacOS" "$id_generator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-id-generator "$id_generator_plugin/Contents/MacOS/clipkeep-plugin-id-generator"
install -m 644 plugins/id-generator/plugin.toml "$id_generator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-IdGenerator-Info.plist "$id_generator_plugin/Contents/Info.plist"
install -d "$id_inspector_plugin/Contents/MacOS" "$id_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-id-inspector "$id_inspector_plugin/Contents/MacOS/clipkeep-plugin-id-inspector"
install -m 644 plugins/id-inspector/plugin.toml "$id_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-IdInspector-Info.plist "$id_inspector_plugin/Contents/Info.plist"
install -d "$json_query_plugin/Contents/MacOS" "$json_query_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-json-query "$json_query_plugin/Contents/MacOS/clipkeep-plugin-json-query"
install -m 644 plugins/json-query/plugin.toml "$json_query_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-JSONQuery-Info.plist "$json_query_plugin/Contents/Info.plist"
install -d "$jwt_inspector_plugin/Contents/MacOS" "$jwt_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-jwt-inspector "$jwt_inspector_plugin/Contents/MacOS/clipkeep-plugin-jwt-inspector"
install -m 644 plugins/jwt-inspector/plugin.toml "$jwt_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-JWTInspector-Info.plist "$jwt_inspector_plugin/Contents/Info.plist"
install -d "$kill_process_plugin/Contents/MacOS" "$kill_process_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-kill-process "$kill_process_plugin/Contents/MacOS/clipkeep-plugin-kill-process"
install -m 644 plugins/kill-process/plugin.toml "$kill_process_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-KillProcess-Info.plist "$kill_process_plugin/Contents/Info.plist"
install -d "$menu_bar_plugin/Contents/MacOS" "$menu_bar_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-menu-bar "$menu_bar_plugin/Contents/MacOS/clipkeep-plugin-menu-bar"
install -m 644 plugins/menu-bar/plugin.toml "$menu_bar_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-MenuBar-Info.plist "$menu_bar_plugin/Contents/Info.plist"
install -d "$network_calculator_plugin/Contents/MacOS" "$network_calculator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-network-calculator "$network_calculator_plugin/Contents/MacOS/clipkeep-plugin-network-calculator"
install -m 644 plugins/network-calculator/plugin.toml "$network_calculator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-NetworkCalculator-Info.plist "$network_calculator_plugin/Contents/Info.plist"
install -d "$password_generator_plugin/Contents/MacOS" "$password_generator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-password-generator "$password_generator_plugin/Contents/MacOS/clipkeep-plugin-password-generator"
install -m 644 plugins/password-generator/plugin.toml "$password_generator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-PasswordGenerator-Info.plist "$password_generator_plugin/Contents/Info.plist"
install -d "$permission_calculator_plugin/Contents/MacOS" "$permission_calculator_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-permission-calculator "$permission_calculator_plugin/Contents/MacOS/clipkeep-plugin-permission-calculator"
install -m 644 plugins/permission-calculator/plugin.toml "$permission_calculator_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-PermissionCalculator-Info.plist "$permission_calculator_plugin/Contents/Info.plist"
install -d "$port_inspector_plugin/Contents/MacOS" "$port_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-port-inspector "$port_inspector_plugin/Contents/MacOS/clipkeep-plugin-port-inspector"
install -m 644 plugins/port-inspector/plugin.toml "$port_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-PortInspector-Info.plist "$port_inspector_plugin/Contents/Info.plist"
install -d "$quick_notes_plugin/Contents/MacOS" "$quick_notes_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-quick-notes "$quick_notes_plugin/Contents/MacOS/clipkeep-plugin-quick-notes"
install -m 644 plugins/quick-notes/plugin.toml "$quick_notes_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-QuickNotes-Info.plist "$quick_notes_plugin/Contents/Info.plist"
install -d "$randomizer_plugin/Contents/MacOS" "$randomizer_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-randomizer "$randomizer_plugin/Contents/MacOS/clipkeep-plugin-randomizer"
install -m 644 plugins/randomizer/plugin.toml "$randomizer_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Randomizer-Info.plist "$randomizer_plugin/Contents/Info.plist"
install -d "$regex_tester_plugin/Contents/MacOS" "$regex_tester_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-regex-tester "$regex_tester_plugin/Contents/MacOS/clipkeep-plugin-regex-tester"
install -m 644 plugins/regex-tester/plugin.toml "$regex_tester_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-RegexTester-Info.plist "$regex_tester_plugin/Contents/Info.plist"
install -d "$reminders_plugin/Contents/MacOS" "$reminders_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-reminders "$reminders_plugin/Contents/MacOS/clipkeep-plugin-reminders"
install -m 644 plugins/reminders/plugin.toml "$reminders_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-Reminders-Info.plist "$reminders_plugin/Contents/Info.plist"
install -d "$system_commands_plugin/Contents/MacOS" "$system_commands_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-system-commands "$system_commands_plugin/Contents/MacOS/clipkeep-plugin-system-commands"
install -m 644 plugins/system-commands/plugin.toml "$system_commands_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-SystemCommands-Info.plist "$system_commands_plugin/Contents/Info.plist"
install -d "$system_status_plugin/Contents/MacOS" "$system_status_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-system-status "$system_status_plugin/Contents/MacOS/clipkeep-plugin-system-status"
install -m 644 plugins/system-status/plugin.toml "$system_status_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-SystemStatus-Info.plist "$system_status_plugin/Contents/Info.plist"
install -d "$text_diff_plugin/Contents/MacOS" "$text_diff_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-text-diff "$text_diff_plugin/Contents/MacOS/clipkeep-plugin-text-diff"
install -m 644 plugins/text-diff/plugin.toml "$text_diff_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-TextDiff-Info.plist "$text_diff_plugin/Contents/Info.plist"
install -d "$text_tools_plugin/Contents/MacOS" "$text_tools_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-text-tools "$text_tools_plugin/Contents/MacOS/clipkeep-plugin-text-tools"
install -m 644 plugins/text-tools/plugin.toml "$text_tools_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-TextTools-Info.plist "$text_tools_plugin/Contents/Info.plist"
install -d "$time_date_plugin/Contents/MacOS" "$time_date_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-time-date "$time_date_plugin/Contents/MacOS/clipkeep-plugin-time-date"
install -m 644 plugins/time-date/plugin.toml "$time_date_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-TimeDate-Info.plist "$time_date_plugin/Contents/Info.plist"
install -d "$unicode_inspector_plugin/Contents/MacOS" "$unicode_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-unicode-inspector "$unicode_inspector_plugin/Contents/MacOS/clipkeep-plugin-unicode-inspector"
install -m 644 plugins/unicode-inspector/plugin.toml "$unicode_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-UnicodeInspector-Info.plist "$unicode_inspector_plugin/Contents/Info.plist"
install -d "$unit_converter_plugin/Contents/MacOS" "$unit_converter_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-unit-converter "$unit_converter_plugin/Contents/MacOS/clipkeep-plugin-unit-converter"
install -m 644 plugins/unit-converter/plugin.toml "$unit_converter_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-UnitConverter-Info.plist "$unit_converter_plugin/Contents/Info.plist"
install -d "$url_inspector_plugin/Contents/MacOS" "$url_inspector_plugin/Contents/Resources"
install -m 755 target/release/clipkeep-plugin-url-inspector "$url_inspector_plugin/Contents/MacOS/clipkeep-plugin-url-inspector"
install -m 644 plugins/url-inspector/plugin.toml "$url_inspector_plugin/Contents/Resources/plugin.toml"
install -m 644 packaging/macos/Plugin-URLInspector-Info.plist "$url_inspector_plugin/Contents/Info.plist"
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
        --identifier io.clipkeep.ClipKeep.base-converter \
        "$base_converter_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.caffeinate \
        "$caffeinate_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.clipboard \
        "$clipboard_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.color-tools \
        "$color_tools_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.cron-schedule \
        "$cron_schedule_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.csv-inspector \
        "$csv_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.date-calculator \
        "$date_calculator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.dns-lookup \
        "$dns_lookup_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.encoding-inspector \
        "$encoding_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.file-checksum \
        "$file_checksum_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.file-info \
        "$file_info_plugin"
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
        --identifier io.clipkeep.ClipKeep.http-inspector \
        "$http_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.id-generator \
        "$id_generator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.id-inspector \
        "$id_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.json-query \
        "$json_query_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.jwt-inspector \
        "$jwt_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.kill-process \
        "$kill_process_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.menu-bar \
        "$menu_bar_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.network-calculator \
        "$network_calculator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.password-generator \
        "$password_generator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.permission-calculator \
        "$permission_calculator_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.port-inspector \
        "$port_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.quick-notes \
        "$quick_notes_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.randomizer \
        "$randomizer_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.regex-tester \
        "$regex_tester_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.reminders \
        "$reminders_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.system-commands \
        "$system_commands_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.system-status \
        "$system_status_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.text-diff \
        "$text_diff_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.text-tools \
        "$text_tools_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.time-date \
        "$time_date_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.unicode-inspector \
        "$unicode_inspector_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.unit-converter \
        "$unit_converter_plugin"
    codesign --force --sign "$signing_identity" --timestamp=none \
        --identifier io.clipkeep.ClipKeep.url-inspector \
        "$url_inspector_plugin"
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
