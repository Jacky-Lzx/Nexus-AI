# Nexus

Nexus is a local-first application launcher and clipboard history palette.
Its native Swift/AppKit interface talks to process-isolated Rust plugins through
a versioned JSON protocol. The current GUI is optimized for macOS; the Rust
plugin protocol remains portable to future Windows and Linux frontends.

## Features

- Watches text, rich text, images, URLs, and file lists while the app is running
- Searches installed macOS applications and opens the selected result
- Searches Google, Bing, DuckDuckGo, GitHub, YouTube, or Google Maps from the
  global palette with explicit prefixes such as `g`, `gh`, `yt`, and `maps`
- Evaluates calculator expressions with parentheses, constants, powers, and
  common functions, then copies or pastes the result
- Converts arbitrarily large signed integers between bases 2–36, including
  familiar binary, octal, decimal, hexadecimal, and base-36 forms
- Converts Unix permission modes between octal and symbolic notation, explains
  each access class and special bit, and accepts `chmod 755` or `perm rwxr-xr-x`
- Validates five-field cron expressions and previews the next five local runs,
  including lists, ranges, steps, names, macros, and standard cron day semantics
- Converts length, mass, temperature, time, and data units with queries such as
  `10 km to mi`, then copies or pastes the result
- Transforms local text with explicit commands for case conversion, URL and
  Base64 encoding, JSON formatting, SHA-256, trimming, and text statistics
- Converts identifiers and phrases among camelCase, PascalCase, snake_case,
  kebab-case, CONSTANT_CASE, dot.case, title case, and lowercase words
- Encodes HTML-sensitive characters, optionally emits ASCII-only numeric
  references, and strictly decodes common named and Unicode numeric entities
- Creates percent-encoded or Base64 text Data URIs and decodes bounded textual
  Data URIs with media-type, parameter, and charset validation
- Inspects Unicode scalars, UTF-8 and UTF-16 encodings, escapes, and invisible
  controls, and decodes sequences such as `unicode U+1F468 U+200D U+1F4BB`
- Shows local and international times using the macOS time-zone database, and
  creates or interprets Unix second and millisecond timestamps
- Adds or subtracts calendar days, weeks, months, years, and weekdays, and
  compares two ISO dates with explicit weekend-only business-day semantics;
  try `datecalc 2026-01-31 + 1 month`, `datecalc today + 10 weekdays`, or
  `datecalc diff 2026-01-01 2026-12-31`
- Calculates SHA-256 and SHA-512 checksums for local files with streaming reads,
  verifies expected digests, and offers explicitly labeled legacy SHA-1 support
- Inspects local file, folder, and symbolic-link metadata, including sizes, Unix
  modes, ownership IDs, timestamps, allocation, inode, and link target status
- Inspects bounded local files for ASCII, UTF-8, UTF-16, UTF-32, BOMs, likely
  BOM-less Unicode, common binary signatures, controls, and mixed line endings
- Reads local battery, disk, memory, CPU, uptime, and network status without
  contacting an external service
- Saves searchable local quick notes with pin, copy, paste, and confirmed-delete
  actions
- Parses form query strings, preserves repeated parameters, emits canonical
  percent encoding and JSON, and builds query strings from flat JSON objects
- Escapes text as JSON string literals or fragments, optionally produces
  ASCII-only Unicode escapes, and strictly decodes JSON escape sequences
- Converts HEX, RGB, HSL, alpha, and common named colors entirely locally
- Generates UUID v4, time-ordered UUID v7, ULID, and configurable Nano ID values
  using the system secure random source
- Validates and normalizes UUID and ULID values, explains UUID version and
  variant fields, and extracts embedded UUID v1/v6/v7 or ULID timestamps
- Generates strong, readable, or alphanumeric passwords and numeric PINs locally
  with unbiased system-secure randomness
- Rolls dice, flips coins, samples integer ranges, picks options, and shuffles
  lists locally with unbiased system-secure randomness
- Decodes JWT headers and payloads locally, formats their JSON, and reports
  unverified time-claim status without implying signature validation
- Calculates IPv4 and IPv6 CIDR networks, masks, endpoint ranges, and address
  counts entirely offline
- Normalizes MAC-48 addresses, explains individual/group and universal/local
  bits, derives Modified EUI-64 interface IDs, and generates secure local addresses
- Summarizes numeric datasets locally with compensated sums, mean, median,
  quartiles, range, and population or sample standard deviation
- Resolves DNS A, AAAA, CNAME, MX, TXT, NS, SRV, CAA, SOA, and reverse PTR
  records through the configured system resolver, including TTLs
- Inspects visible local TCP listeners and UDP bindings by port, process, PID,
  user, endpoint, and socket state without changing or terminating them
- Tests Rust regular expressions locally with common flags, match positions,
  numbered and named capture groups, and bounded reports
- Parses, compares, and increments Semantic Versioning 2.0.0 values locally,
  including specification-correct prerelease precedence and build metadata
- Inspects bounded CSV, TSV, semicolon, and pipe-delimited data locally, profiles
  column types and irregular records, and converts tables to CSV, TSV, or JSON
- Compares two text values locally by line, word token, or Unicode scalar and
  produces a bounded, copyable diff with addition and removal counts
- Sorts, reverses, deduplicates, and counts frequencies in bounded multiline
  text, with optional trimming, blank removal, and lowercase-based matching
- Queries local JSON with JSON Pointer or dot/bracket paths, lists immediate
  child keys, and enumerates bounded leaf-value paths
- Inspects and normalizes URLs locally, including credentials, hosts, ports,
  decoded paths, duplicate query parameters, and fragments
- Reads and searches upcoming EventKit calendar events, with a configurable
  1–90 day range and on-demand calendar permission
- Runs common macOS commands including Lock Screen, display sleep, screen saver,
  system sleep, Settings, Force Quit, log out, restart, and shut down
- Reads and switches Clash/Mihomo Rule, Global, and Direct modes through the
  configured External Controller API
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
- No analytics or cloud service; network access is limited to explicit features
  such as DNS and HTTP inspection, web search, and the configured Clash controller
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

Query JSON locally with `jget user.name :: {"user":{"name":"Ada"}}`, list
immediate children with `jkeys /user :: <json>`, or enumerate leaf values with
`jpaths :: <json>`. Paths accept JSON Pointer as well as dot and array-index syntax.

Compare pasted text with `diff old text :: new text`, switch to word-token output
with `diff words hello, world :: hello world!`, or use `diff chars` for a
Unicode-scalar comparison. The report marks unchanged, removed, and added units
and can be copied or pasted into the previously focused application.

Inspect delimited data with `csv :: name,age` followed by pasted rows, or use
`tsv no-header ::` for tab-separated data without a header. Options `comma`,
`tab`, `semicolon`, and `pipe` select the delimiter. Results profile column types,
flag inconsistent record widths, preview bounded rows, and convert the parsed
table to normalized CSV, TSV, or JSON entirely locally.

Hash a local file with `checksum ~/Downloads/archive.zip`, request one algorithm
with `checksum sha256 <absolute-path>`, or verify a published digest with
`checksum verify sha256 <digest> <absolute-path>`. SHA-1 remains available only
for compatibility and is labeled as a legacy algorithm in every result.

Inspect metadata without reading file contents with `fileinfo ~/Downloads/archive.zip`.
The result distinguishes files, folders, links, sockets, pipes, and devices; shows
Unix mode, size/allocation, ownership IDs, timestamps, inode, and link status; and
offers copy, open, and Finder reveal actions.

Inspect a file's text encoding with `encoding ~/Downloads/data.txt`. Files up to
16 MiB are checked in full for Unicode validity, byte-order marks, likely BOM-less
UTF-16/32, common binary signatures, NUL/control bytes, line-ending styles, and a
final newline. The plugin reports inference as heuristic rather than definitive.

Inspect a version with `semver 1.2.3-beta.2+build.7`, compare precedence with
`semver compare 1.2.3-rc.1 1.2.3`, or create a new version with
`semver bump minor 1.2.3`. `bump release` removes prerelease and build metadata
without incrementing the core version. A conventional leading `v` is accepted
as input and removed from the normalized result.

Summarize pasted numbers with `stats 1, 2, 3, 4.5`. Commas, semicolons, and
whitespace are accepted between values; the report includes the sum, mean,
median, linearly interpolated quartiles, range, and both population and sample
standard deviation for up to 10,000 finite values.

Inspect a MAC-48 address with `macaddr 00:11:22:33:44:55`; compact, hyphenated,
and Cisco dotted forms are also accepted. The result explains unicast/multicast
and universal/local bits, provides normalized forms and Modified EUI-64 output,
and does not guess a vendor without an OUI database. Use `macaddr random` to
create a cryptographically secure locally administered unicast address.

Clean a pasted list with `linetools sort fold trim nonempty ::` followed by the
multiline text. Operations `sort`, `unique`, `sort-unique`, `reverse`, and
`frequency` normalize line endings to LF while preserving a final newline;
`asc` or `desc` selects ordering, and `fold` uses Unicode lowercase matching.

Convert a name with `caseconv snake :: XMLHttpRequest2`, or omit the target with
`caseconv :: user profile ID` to see all supported styles. The converter detects
separator, lowercase-to-uppercase, acronym, and letter/number boundaries while
retaining Unicode alphanumeric words.

Escape markup with `htmlencode :: <p title="A & B">café</p>`, or use
`htmlencode ascii :: café` to emit non-ASCII scalars as hexadecimal numeric
references. `htmldecode :: &lt;p&gt;&#x1F600;&lt;/p&gt;` strictly decodes
semicolon-terminated numeric references and a documented core of common named
entities while leaving ordinary literal ampersands intact.

Create a text Data URI with `datauri :: Hello, 世界`, select Base64 and a media
type with `datauri base64 application/json :: {"ready":true}`, or recover text
with `datauridecode :: data:text/plain;charset=UTF-8,Hello%20world`. Decoding
validates metadata and UTF-8/US-ASCII declarations and rejects binary payloads.

Normalize form parameters with `querystring :: name=Alice+Smith&tag=rust&tag=mac`;
the plugin returns both a canonical RFC 3986 query and grouped JSON without
dropping repeated values. Reverse the conversion with
`queryjson :: {"name":"Alice Smith","tag":["rust","mac"]}`.

Produce a complete JSON string literal with `jsonescape :: line "one"`, omit
the surrounding quotes with `jsonescape fragment :: line "one"`, or force portable
ASCII-only escapes with `jsonescape ascii :: café 😀`. Decode strict literals
or content fragments with `jsonunescape :: "line\\nnext"` and
`jsonunescape fragment :: line\\nnext`.

Inspect an existing identifier with `idinfo 550e8400-e29b-41d4-a716-446655440000`,
or force its type with `uuidinfo` or `ulidinfo`. UUIDs are normalized and decoded
by version and variant; UUID v1/v6/v7 and ULID values expose their embedded UTC
timestamp and Unix milliseconds without making any network request.

Resolve both address families with `dns example.com`, request a specific record
with `dns mx example.com` or `dns txt _dmarc.example.com`, and perform reverse
lookup by entering an address such as `dns 1.1.1.1`. Results show the configured
resolver, response status, TTL, and query time.

Inspect a website without downloading its response body with `http example.com`.
The result follows up to five HTTP/HTTPS redirects and reports the final status,
protocol, remote peer, content type, header size, certificate verification, and
DNS/connect/TLS/first-byte/total timings. URLs containing credentials are rejected.

Convert integers with `base 0xff`, `base 16:deadbeef to 2`, or
`base z from 36 to decimal`. Without a target, Nexus shows binary, octal,
decimal, hexadecimal, and base-36 results together.

Open `Clash` to inspect and switch between Rule, Global, and Direct modes. Configure
the HTTP `external-controller` address and optional `secret` in the Clash settings
pane. Clash Verge Rev is detected through `/tmp/verge/verge-mihomo.sock` first;
the default HTTP fallback is `http://127.0.0.1:9090`.

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
content. Nexus never uploads clipboard history. Network access occurs only for
explicit features such as DNS lookup, HTTP inspection, Clash control, and web
search. History is encrypted at rest, concealed/transient pasteboard data is
ignored by default, and additional applications can be excluded by bundle
identifier in Settings.

## Architecture

- `crates/clipkeep-protocol`: stable, versioned host/plugin message types
- `crates/clipkeep-plugin-sdk`: Rust plugin runtime over newline-delimited JSON
- `swift/Sources/ClipKeep`: native AppKit panel, animations, status item, global
  shortcut, plugin process lifecycle, result aggregation, and action routing
- `crates/clipkeep-core`: reusable Rust host implementation retained for future
  non-macOS frontends and protocol tests
- `plugins/applications`: installed application index and launcher
- `plugins/base-converter`: arbitrary-precision integer radix conversion
- `plugins/caffeinate`: timed display and system sleep prevention with cancellation
- `plugins/clipboard`: clipboard capture, search, persistence, and actions
- `plugins/color-tools`: local CSS color parsing and format conversion
- `plugins/cron-schedule`: cron validation and upcoming local-run previews
- `plugins/csv-inspector`: bounded local delimited-data inspection and conversion
- `plugins/data-uri`: local text Data URI generation and validated decoding
- `plugins/date-calculator`: local Gregorian date arithmetic and differences
- `plugins/dns-lookup`: explicit DNS record and reverse-address lookups
- `plugins/encoding-inspector`: bounded local Unicode, binary-signature, and line-ending inspection
- `plugins/file-checksum`: streaming local file hashes and digest verification
- `plugins/file-info`: local file, directory, and symbolic-link metadata inspection
- `plugins/calculator`: expression parser and result copy/paste actions
- `plugins/calendar`: read-only EventKit upcoming-event search
- `plugins/case-converter`: local identifier and phrase naming-style conversion
- `plugins/clash`: Clash/Mihomo External Controller mode status and switching
- `plugins/files`: Spotlight-backed file and folder search with open and Finder reveal actions
- `plugins/html-entities`: local HTML entity encoding and strict decoding
- `plugins/http-inspector`: bounded explicit HTTP HEAD requests with redirect and timing details
- `plugins/id-generator`: secure local UUID, ULID, and Nano ID generation
- `plugins/id-inspector`: local UUID and ULID validation, normalization, and timestamp decoding
- `plugins/json-query`: local JSON value, child-key, and leaf-path queries
- `plugins/json-string`: local JSON string escaping and strict unescaping
- `plugins/jwt-inspector`: local JWT header, payload, and time-claim inspection
- `plugins/kill-process`: process-name search and confirmed SIGTERM termination
- `plugins/line-tools`: bounded multiline sorting, deduplication, reversal, and frequencies
- `plugins/mac-address`: local MAC-48 inspection, normalization, EUI-64, and generation
- `plugins/menu-bar`: searchable left-side application menus through Accessibility
- `plugins/network-calculator`: offline IPv4 and IPv6 CIDR/subnet calculation
- `plugins/number-statistics`: bounded local descriptive statistics for numeric datasets
- `plugins/password-generator`: secure local password and PIN generation
- `plugins/permission-calculator`: local Unix chmod conversion and explanation
- `plugins/port-inspector`: read-only local TCP listener and UDP binding inspection
- `plugins/query-string`: local query-string normalization and JSON conversion
- `plugins/quick-notes`: persistent, prefix-triggered local notes and snippets
- `plugins/randomizer`: secure dice, coin, integer, choice, and shuffle tools
- `plugins/regex-tester`: bounded local Rust regular-expression testing and captures
- `plugins/reminders`: persistent reminders and background notification delivery
- `plugins/semantic-version`: local SemVer parsing, precedence comparison, and increments
- `plugins/system-commands`: curated macOS system actions with confirmation for
  disruptive commands
- `plugins/system-status`: read-only local hardware and operating-system status
- `plugins/text-diff`: bounded local line, word-token, and Unicode-scalar diffs
- `plugins/text-tools`: local text transformations with copy/paste actions
- `plugins/time-date`: world clocks and Unix timestamp conversion
- `plugins/unicode-inspector`: Unicode scalar, encoding, and escape inspection
- `plugins/unit-converter`: natural unit conversion with copy/paste actions
- `plugins/url-inspector`: local URL normalization and component inspection
- `plugins/web-search`: prefix-triggered searches opened in the default browser

Built-in plugins are separate executables. A plugin crash cannot directly crash
the UI. A crashed plugin is restarted with bounded backoff, while disabling it
stops the process and unregisters its shortcut.

Plugins can also publish compact presentation items for their dedicated search page.
The host renders their icon, title, value, and semantic style consistently; countdown
metadata is updated locally by the GUI without continuously querying the plugin.
Actions may declare typed inline input fields with placeholders and numeric bounds.
The AppKit host renders them in the search bar, provides Tab/Shift-Tab navigation,
and returns their values to the Rust plugin as JSON when the user presses Return.

Applications and System Commands expose their complete item catalogs in Settings.
Each application or command can be hidden independently; hidden items are filtered
inside the Rust plugin and therefore disappear from both global and dedicated
search results.

## Plugin bundles

Plugins run as separate executables and exchange newline-delimited JSON with Nexus
on stdin/stdout. Rust plugins can use `clipkeep-plugin-sdk`; other languages can
implement the same protocol. No dynamic linking or host rebuild is required.

### Install, update, and remove

Press **Command+,** in the launcher, then open **Plugins → Install Plugin…** and select a `.clipkeepplugin`
package. Select a newer package with the same ID to update it. User-installed
packages override bundled packages with the same ID, regardless of version;
installing an older version is also allowed. Updates preserve preferences and data.

Select an installed plugin in the settings sidebar and choose **Uninstall…** to
move its package to Trash. Data and preferences remain intact. If an override is
removed, the bundled version becomes available again. Bundled packages are part
of the signed application and can be disabled with their enable switch.

Nexus also scans this directory every second while running:

```text
~/Library/Application Support/io.clipkeep.ClipKeep/Plugins
```

You can add/remove complete plugin directories there. For manual updates, stage
the complete package outside this directory and move it in; do not copy files
into a running plugin piecemeal. The installer uses an atomic directory exchange.
New, changed, and removed plugins update running processes, search results,
shortcuts, status menus, and settings without restarting Nexus. Actions captured
from retired processes are rejected. An empty plugin directory stays empty.

The application also discovers bundled plugins in `Contents/PlugIns`. Invalid
manifests, missing executables, unsupported protocol versions, and duplicate IDs
within a directory are reported in launcher status. Within one directory the
first valid package by filename wins duplicate-ID conflicts. Executables must
resolve inside their own bundle.

Plugins are native programs running with the user's privileges. Capability names
select host integrations; they are not an operating-system sandbox or a security
review. Install only packages you trust.

### Build a plugin independently

The app release script still builds the bundled plugins. To build just one plugin
without rebuilding or re-signing Nexus (Python 3.11+ and Rust required):

```sh
./scripts/package-plugin.py plugins/calculator
./scripts/package-plugin.py examples/hello-plugin
```

Packages are written to `dist/plugins/`. Use `--output <artifact-directory>` to
choose a different output folder and `--identity "Your Signing Identity"` (or
`CLIPKEEP_CODESIGN_IDENTITY`) to sign with your certificate. Otherwise packages
receive an ad-hoc signature for local development. Public distribution needs the
appropriate Developer ID signing/notarization. The output directory is for build
artifacts; install the completed package using Nexus rather than building into
the live plugin directory. If dependencies are cached, `CARGO_NET_OFFLINE=true`
can be used for an offline build.

`examples/hello-plugin` is a separate Cargo workspace, intentionally absent from
Nexus's workspace members. It demonstrates independent compilation and an action
that returns a notification. Copy this structure for a new plugin, choose a unique
ID, and point its SDK dependencies at your Nexus source checkout. After installing
it, search for `hello` to verify it works.

A package can be a flat directory with `plugin.toml` and its executable, or a macOS
bundle containing `Contents/Resources/plugin.toml` and `Contents/MacOS/<entrypoint>`.
Optional source files under `resources/` are included by the packaging script.

Example manifest (flat scalar fields, double-quoted strings, single-line arrays):

```toml
id = "com.example.nexus.hello"
name = "Hello Plugin"
description = "An independently built example plugin"
icon = "hand.wave"
version = "0.1.0"
protocol_version = 1
entrypoint = "nexus-plugin-hello"
default_enabled = true
include_in_global_search = true
capabilities = ["search"]
```

The manifest's entrypoint must match a Cargo binary target. `description` and
`icon` (an SF Symbol name) are optional. Strings/arrays use JSON-compatible quoted
syntax; TOML tables, literal strings, and multiline arrays are not supported.

Discovered plugins receive an enable switch, configurable shortcut, main-search
item name, global-search inclusion switch, menu entry, and isolated process
lifecycle. Plugins excluded from Global Search receive searchable route items
that open dedicated search windows. Clipboard History remains a dedicated mode.
Declare `configuration.items` and implement `configuration_items` in the SDK to
show a per-item visibility list; consume the `hidden_items` configure setting.

### Host services

Most actions are sent to the plugin's `execute` method. An action can instead
request a host integration by adding `host_action.<action-id>` to its result's
`metadata`. Declare the corresponding capability in `plugin.toml`:

| Metadata value | Required capability | Additional result metadata / behavior |
| --- | --- | --- |
| `system.media` | `system.control` | `command`: `mute`, `volume-up`, `volume-down`, `play-pause`, `next-track`, or `previous-track` |
| `menu.press` | `accessibility.press` | `target_pid`, `menu_path` (JSON array), `menu_item_id`; performs the selected application's menu command |
| `text.edit` | `clipboard.write` | Opens the external text editor using `preview`; submits edited text to `edit_submit_action` (defaults to `apply_edit`) |
| `form.reminder` | `notifications` | Shows a content/date form, then calls the original action with JSON `content` and `due_at` input |

For example, `metadata = {"host_action.toggle": "system.media", "command": "mute"}`
requests a host media action for an action whose ID is `toggle`. New plugin IDs
need no host code changes. Unknown services or missing capability declarations
fail explicitly. Old bundled protocol-v1 actions retain a compatibility adapter.
Plugins with `accessibility.read` receive `target_pid` and serialized menu `items`
through runtime configure messages when their search context is opened.

The built-in Clipboard, Calendar, and Clash settings retain their specialized
settings pages. Generic plugins use manifest presentation, action input fields,
and optional configuration item lists.

Run `dist/Nexus.app/Contents/MacOS/Nexus --list-plugins` to inspect discovery.
Plugin stderr logs are written under the app-support `Logs` directory.

### Inline command arguments

Include a plugin in global search (`include_in_global_search = true`) and return a
command whose default action declares `input_fields`. On the main page, when this
command is the first selected result for a nonempty query, its argument fields
appear beside the query without taking focus. Press Tab to enter the first field.
Editing the query or selecting another result updates the argument preview. Tab and
Shift-Tab cycle through fields, Return submits, and Escape restores the query.
Tab waits for the current search to finish, just like Return. Users can override
Global Search inclusion in Settings.

Each `ActionInputField` declares a unique `id`, `label`, optional `placeholder`,
`kind` (`text` or `integer`), `required`, and optional integer `minimum`/`maximum`.
The host validates the fields and passes a JSON object **encoded as a string** in
`HostRequest::Execute.input`; implement the SDK's `execute_with_input` to consume
it. Optional empty integer fields become `0`; empty text fields become `""`.
Plugins must also validate input themselves and report errors without starting an
invalid operation. Existing actions without fields keep their normal behavior.

For example, Caffeinate returns `hours` (label `hour`, integer 0–8760) and `minutes`
(label `min`, integer 0–59). Type `caffeinate`, Tab, `1`, Tab, `30`, Return to start
90 minutes. The plugin receives `{"hours":1,"minutes":30}`. A zero total is rejected.
See `plugins/caffeinate/src/main.rs` for the complete reusable action declaration
and execution handler; no plugin-specific host UI is needed.


### OCR clipboard images

Copy an image (for example, take a macOS screenshot to the clipboard), search
`ocr`, and press Return on **OCR Clipboard Image**. Recognition runs locally in
the independent plugin using Apple's Vision framework, with Chinese and English
recognition and automatic language detection. Search remains available while it
runs. Select **OCR Result** to read the full text in a scrollable preview; Return
copies it. Select **OCR Clipboard Image** again to recognize a new image.

No image, unreadable images, and images with no recognized text produce a visible
message. Inputs are limited to 64 MB and 80 megapixels. Results are held in memory
until replaced or the plugin exits. This first version accepts PNG/TIFF clipboard
images; image-file selection, screen-region capture, and table reconstruction are
not included.

Plugins may set result metadata `preview_style = "text"` to display a selectable,
scrollable plain-text `preview`, including in global search. Copy remains a normal
plugin action.

OCR is a native Swift protocol-v1 plugin in `plugins/ocr`. The packaging script
accepts either a Rust `Cargo.toml` or native Swift `main.swift`:

```sh
python3 scripts/package-plugin.py plugins/ocr
swiftc -module-cache-path target/swift/module-cache plugins/ocr/tests/integration.swift -o /tmp/nexus-ocr-integration
/tmp/nexus-ocr-integration "$PWD/target/swift-plugins/clipkeep-plugin-ocr"
```

The integration test temporarily replaces and restores the clipboard, checking
missing-image errors, responsive searches during OCR, Chinese/English recognition,
copying, query filtering, and shutdown. Run in a normal macOS session so Vision and
the pasteboard are available. The binary also supports `--recognize-file <path>`
for read-only recognizer diagnostics.
