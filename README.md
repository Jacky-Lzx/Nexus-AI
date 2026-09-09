# ClipKeep

ClipKeep is a small, local-first clipboard history manager written in Rust. It
works on macOS today and uses cross-platform libraries that also support Windows
and Linux.

## Features

- Watches the text clipboard while the app is running
- Persists up to 500 entries locally between launches
- Search, favorite, restore, and delete history entries
- Deduplicates repeated clipboard contents
- Floating, borderless, always-on-top command palette
- Keyboard navigation: arrows, Enter, `Command-S`, and `Command-Backspace`
- No network access or analytics

## Run

Install a current stable Rust toolchain, then:

```sh
cargo run --release
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
sends data over the network, but its history file is currently plain JSON. Use
**Clear all** to remove all non-favorite entries.
