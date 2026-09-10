# Nexus

Nexus is a local-first application launcher and clipboard history palette.
Its native Swift/AppKit interface talks to process-isolated Rust plugins through
a versioned JSON protocol. The current GUI is optimized for macOS; the Rust
plugin protocol remains portable to future Windows and Linux frontends.

## Features

- Watches text, rich text, images, URLs, and file lists while the app is running
- Searches installed macOS applications and opens the selected result
- Evaluates calculator expressions with parentheses, constants, powers, and
  common functions, then copies or pastes the result
- Reads and searches upcoming EventKit calendar events, with a configurable
  1–90 day range and on-demand calendar permission
- Runs common macOS commands including Lock Screen, display sleep, screen saver,
  system sleep, Settings, Force Quit, log out, restart, and shut down
- Searches running processes by name and sends SIGTERM only
  after an explicit confirmation containing the process name and PID
- Preserves native pasteboard representations such as UTF-8, RTF, HTML,
  PNG, TIFF, file URLs, and multiple pasteboard items
- Generates compact image thumbnails in Rust for list and detail previews
- Persists up to 500 entries locally between launches, encrypted with AES-256-GCM
  using a key stored in the macOS Keychain, with per-item and total storage limits
- Search, favorite, restore, and delete history entries
- Deduplicates repeated clipboard contents
- Native Liquid Glass, rounded, borderless, always-on-top command palette
- Starts as a compact search bar and smoothly expands only while searching
- Clipboard mode opens expanded and immediately shows the existing history
- Animated fade-in and fade-out when showing or hiding
- Keyboard navigation: arrows, Enter, `Command-S`, and `Command-Backspace`
- Plugin-defined action menus on right-click or `Command-K`; action behavior such
  as closing the palette and pasting into the previous app is protocol-driven
- `Command-Shift-Space` opens a combined search across opted-in plugins
- `Command-Shift-C` searches clipboard history only
- In Clipboard History, `Enter` pastes immediately; `Command-Enter` starts an asynchronous Kitty/Neovim edit session and pastes the saved text after the editor closes
- Kitty and Neovim executable paths can be configured per Clipboard History plugin; empty paths use automatic discovery
- Automatically hides when focus moves to another application
- Escape hides the palette and restores focus to the previously active application
- Choosing a clipboard entry restores the previous application and pastes it directly
- Plugins keep running while the window is hidden
- Persistent reminders with a native content/time form and macOS notifications
- Discovers versioned plugin bundles from the app bundle and the user plugin
  directory, with manifest validation, per-plugin logs, search timeouts, and
  bounded automatic restart after a crash
- No network access or analytics
- Configurable history retention, sensitive-content filtering, ignored source
  applications, and time-range clearing (favorites are preserved)

## Run

Install a current stable Rust toolchain and the macOS Swift toolchain, then:

```sh
cargo build --workspace
./scripts/build-swift.sh
```

To create a normal macOS application bundle that can be moved into
`/Applications`:

```sh
chmod +x scripts/package-macos.sh
./scripts/package-macos.sh
open dist/Nexus.app
```

For UI testing or scripted launch, an initial search can be supplied with
`open -n dist/Nexus.app --args --query Safari`.

The generated bundle is unsigned. On another Mac, Gatekeeper may require the
user to right-click the app and choose **Open**. Public distribution should add
Developer ID signing and notarization.

Direct paste requires macOS Accessibility permission so Nexus can send the
`Command-V` keystroke to the previously focused application. The first direct
paste attempt opens the system permission prompt; once enabled, choose the entry
again to paste it.

### Keep Accessibility permission across local builds

macOS cannot preserve privacy permissions for ad-hoc signed builds because their
code identity changes whenever the executable changes. Create a development
code-signing identity in Xcode or Keychain Access, then package the app normally:

```sh
./scripts/package-macos.sh
```

The private key remains in the login keychain. The packaging script automatically
uses the identity named `ClipKeep Local Development` to sign the app and its Rust
plugin executables. Grant Accessibility permission once more after switching from
the old ad-hoc build; subsequent signed builds retain the same designated requirement.

To use an Apple Development or Developer ID identity instead, set
`CLIPKEEP_CODESIGN_IDENTITY` to its certificate name when packaging.

Nexus requires macOS 26 or later. Rust plugin executables can remain
cross-platform even when another native frontend is added later.

Press `Command-Shift-Space` to search applications, calculator results, and
system commands together, or `Command-Shift-C` to search clipboard history.
Search for `plugin` or `plugins` in the main window to open a searchable list of
every enabled plugin, including plugins whose results already participate in global search.
Calendar results can be added to the combined search in Settings; Clipboard is
kept out of it and remains available through its shortcut or the existing
"Clipboard History" result. Escape and focus loss hide the panel without stopping
its plugin processes. Use
**Quit** in the bottom action bar to stop Nexus completely.

Open the `Reminders` item from the main search, choose `Add Reminder`, then enter
the content and a future date and time. Nexus requests notification permission on
the first reminder and continues checking reminders while it runs in the background.
Reminder data is stored locally alongside the other Nexus application data.

Open `Caffeinate`, choose an end time, and Nexus will keep both the display and Mac
awake until then. Run `Decaffeinate` from the same plugin to end the session early.

Open the `Menu Bar` item to search commands from every submenu of the application
that was focused before Nexus opened. Results include their complete menu path, such
as `File › New Window`; selecting one restores the application and performs that
command through macOS Accessibility. Right-side status items are intentionally excluded.

Encrypted history is stored in the platform application-data directory:

- macOS: `~/Library/Application Support/io.clipkeep.ClipKeep/history.ckh`
- Windows: `%APPDATA%\\clipkeep\\ClipKeep\\data\\history.json`
- Linux: `~/.local/share/clipkeep/history.json` (subject to XDG settings)

## Privacy note

Clipboard history can contain passwords, images, files, and other sensitive
content. Nexus never sends data over the network. History is encrypted at
rest, concealed/transient pasteboard data is ignored by default, and additional
applications can be excluded by bundle identifier in Settings.

## Architecture

- `crates/clipkeep-protocol`: stable, versioned host/plugin message types
- `crates/clipkeep-plugin-sdk`: Rust plugin runtime over newline-delimited JSON
- `swift/Sources/ClipKeep`: native AppKit panel, animations, status item, global
  shortcut, plugin process lifecycle, result aggregation, and action routing
- `crates/clipkeep-core`: reusable Rust host implementation retained for future
  non-macOS frontends and protocol tests
- `plugins/applications`: installed application index and launcher
- `plugins/caffeinate`: timed display and system sleep prevention with cancellation
- `plugins/clipboard`: clipboard capture, search, persistence, and actions
- `plugins/calculator`: expression parser and result copy/paste actions
- `plugins/calendar`: read-only EventKit upcoming-event search
- `plugins/kill-process`: process-name search and confirmed SIGTERM termination
- `plugins/menu-bar`: searchable left-side application menus through Accessibility
- `plugins/reminders`: persistent reminders and background notification delivery
- `plugins/system-commands`: curated macOS system actions with confirmation for
  disruptive commands

Built-in plugins are separate executables. A plugin crash cannot directly crash
the UI. A crashed plugin is restarted with bounded backoff, while disabling it
stops the process and unregisters its shortcut.

Plugins can also publish compact presentation items for their dedicated search page.
The host renders their icon, title, value, and semantic style consistently; countdown
metadata is updated locally by the GUI without continuously querying the plugin.

Applications and System Commands expose their complete item catalogs in Settings.
Each application or command can be hidden independently; hidden items are filtered
inside the Rust plugin and therefore disappear from both global and dedicated
search results.

## Plugin bundles

A plugin bundle is a directory ending in `.clipkeepplugin` that contains a Rust
executable and `plugin.toml`. Nexus scans both `Contents/PlugIns` inside the
application and:

```text
~/Library/Application Support/io.clipkeep.ClipKeep/Plugins
```

Example manifest:

```toml
id = "com.example.clipkeep.plugin"
name = "Example"
version = "0.1.0"
protocol_version = 1
entrypoint = "clipkeep-plugin-example"
default_enabled = true
include_in_global_search = true
capabilities = ["search"]
```

Discovered plugins automatically receive a settings pane, enable switch,
configurable shortcut, main-search item name, global-search inclusion switch,
status-menu entry, and isolated process lifecycle. An enabled plugin excluded
from Global Search receives a searchable item in the main page that opens its
dedicated search window. That item is always hidden while the plugin participates
in Global Search. Clipboard History is deliberately excluded from the combined
search regardless of its manifest and is only opened explicitly.
Plugins can optionally implement the SDK's `configuration_items` catalog and
consume the host-provided `hidden_items` setting to offer the same per-item
visibility controls.
Run `Nexus --list-plugins` against a built app to validate discovery. Plugin
stderr logs are written under the app-support `Logs` directory.
