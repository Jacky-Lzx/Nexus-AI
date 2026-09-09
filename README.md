# ClipKeep

ClipKeep is a local-first application launcher and clipboard history palette.
Its native Swift/AppKit interface talks to process-isolated Rust plugins through
a versioned JSON protocol. The current GUI is optimized for macOS; the Rust
plugin protocol remains portable to future Windows and Linux frontends.

## Features

- Watches text, rich text, images, URLs, and file lists while the app is running
- Searches installed macOS applications and opens the selected result
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
- `Command-Shift-Space` searches applications only
- `Command-Shift-C` searches clipboard history only
- Automatically hides when focus moves to another application
- Escape hides the palette and restores focus to the previously active application
- Choosing a clipboard entry restores the previous application and pastes it directly
- Plugins keep running while the window is hidden
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
open dist/ClipKeep.app
```

For UI testing or scripted launch, an initial search can be supplied with
`open -n dist/ClipKeep.app --args --query Safari`.

The generated bundle is unsigned. On another Mac, Gatekeeper may require the
user to right-click the app and choose **Open**. Public distribution should add
Developer ID signing and notarization.

Direct paste requires macOS Accessibility permission so ClipKeep can send the
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

ClipKeep requires macOS 26 or later. Rust plugin executables can remain
cross-platform even when another native frontend is added later.

Press `Command-Shift-Space` to search applications or `Command-Shift-C` to
search clipboard history. Escape and focus loss hide the panel without stopping
its plugin processes. Use
**Quit** in the bottom action bar to stop ClipKeep completely.

Encrypted history is stored in the platform application-data directory:

- macOS: `~/Library/Application Support/io.clipkeep.ClipKeep/history.ckh`
- Windows: `%APPDATA%\\clipkeep\\ClipKeep\\data\\history.json`
- Linux: `~/.local/share/clipkeep/history.json` (subject to XDG settings)

## Privacy note

Clipboard history can contain passwords, images, files, and other sensitive
content. ClipKeep never sends data over the network. History is encrypted at
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
- `plugins/clipboard`: clipboard capture, search, persistence, and actions

Built-in plugins are separate executables. A plugin crash cannot directly crash
the UI, and disabling a plugin can be implemented by stopping its process.
