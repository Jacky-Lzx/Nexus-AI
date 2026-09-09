# ClipKeep

ClipKeep is a local-first application launcher and clipboard history palette.
Its native Swift/AppKit interface talks to process-isolated Rust plugins through
a versioned JSON protocol. The current GUI is optimized for macOS; the Rust
plugin protocol remains portable to future Windows and Linux frontends.

## Features

- Watches the text clipboard while the app is running
- Searches installed macOS applications and opens the selected result
- Persists up to 500 entries locally between launches
- Search, favorite, restore, and delete history entries
- Deduplicates repeated clipboard contents
- Translucent, rounded, borderless, always-on-top command palette
- Starts as a compact search bar and smoothly expands only while searching
- Clipboard mode opens expanded and immediately shows the existing history
- Animated fade-in and fade-out when showing or hiding
- Keyboard navigation: arrows, Enter, `Command-S`, and `Command-Backspace`
- `Command-Shift-Space` searches applications only
- `Command-Shift-C` searches clipboard history only
- Automatically hides when focus moves to another application
- Plugins keep running while the window is hidden
- No network access or analytics

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
open dist/ClipKeep.app
```

For UI testing or scripted launch, an initial search can be supplied with
`open -n dist/ClipKeep.app --args --query Safari`.

The generated bundle is unsigned. On another Mac, Gatekeeper may require the
user to right-click the app and choose **Open**. Public distribution should add
Developer ID signing and notarization.

The AppKit host requires macOS 11 or later. Rust plugin executables can remain
cross-platform even when another native frontend is added later.

Press `Command-Shift-Space` to search applications or `Command-Shift-C` to
search clipboard history. Escape and focus loss hide the panel without stopping
its plugin processes. Use
**Quit** in the bottom action bar to stop ClipKeep completely.

History is stored in the platform application-data directory:

- macOS: `~/Library/Application Support/io.clipkeep.ClipKeep/history.json`
- Windows: `%APPDATA%\\clipkeep\\ClipKeep\\data\\history.json`
- Linux: `~/.local/share/clipkeep/history.json` (subject to XDG settings)

## Privacy note

Clipboard history can contain passwords and other sensitive text. ClipKeep never
sends data over the network, but its history file is currently plain JSON.
Individual clipboard entries can be removed from the result action bar.

## Architecture

- `crates/clipkeep-protocol`: stable, versioned host/plugin message types
- `crates/clipkeep-plugin-sdk`: Rust plugin runtime over newline-delimited JSON
- `swift/Sources/ClipKeep`: native AppKit panel, animations, status item, global
  shortcut, plugin process lifecycle, result aggregation, and action routing
- `crates/clipkeep-core`: reusable Rust host implementation retained for future
  non-macOS frontends and protocol tests
- `plugins/applications`: installed application index and launcher
- `plugins/clipboard`: clipboard capture, search, persistence, and actions

Built-in plugins are separate executables. A plugin crash cannot directly crash
the UI, and disabling a plugin can be implemented by stopping its process.
