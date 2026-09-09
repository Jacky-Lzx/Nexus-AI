# ClipKeep

ClipKeep is a local-first application launcher and clipboard history palette.
The UI talks to process-isolated Rust plugins through a versioned JSON protocol.
It works on macOS today, with the core and plugin architecture designed for
Windows and Linux frontends later.

## Features

- Watches the text clipboard while the app is running
- Searches installed macOS applications and opens the selected result
- Persists up to 500 entries locally between launches
- Search, favorite, restore, and delete history entries
- Deduplicates repeated clipboard contents
- Floating, borderless, always-on-top command palette
- Keyboard navigation: arrows, Enter, `Command-S`, and `Command-Backspace`
- No network access or analytics

## Run

Install a current stable Rust toolchain, then:

```sh
cargo build --workspace
cargo run
```

To create a normal macOS application bundle that can be moved into
`/Applications`:

```sh
chmod +x scripts/package-macos.sh
./scripts/package-macos.sh
open dist/ClipKeep.app
```

The generated bundle is unsigned. On another Mac, Gatekeeper may require the
user to right-click the app and choose **Open**. Public distribution should add
Developer ID signing and notarization.

On Linux, the system may also need the development packages required by `winit`
and X11/Wayland. macOS needs no additional runtime dependencies.

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
- `crates/clipkeep-core`: plugin process lifecycle, concurrent result aggregation,
  ranking, stale-query rejection, and action routing
- `plugins/applications`: installed application index and launcher
- `plugins/clipboard`: clipboard capture, search, persistence, and actions

Built-in plugins are separate executables. A plugin crash cannot directly crash
the UI, and disabling a plugin can be implemented by stopping its process.
