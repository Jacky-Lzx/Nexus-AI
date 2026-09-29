# Plugin protocol evolution

The transport remains newline-delimited JSON, protocol version 1. Existing plugin
bundles remain supported. Changes are delivered in separately validated commits:

1. Correlated errors and terminal search states.
2. Cancellation, deadlines and coalescing queued searches.
3. Typed action arguments, settings and host-service declarations.
4. Feature negotiation and shared Rust/Swift wire fixtures.
5. Bounded transport, diagnostics and startup readiness.

## Search completion

`error` may include `query_id` or `request_id`, `code`, and `retryable`.
A query-scoped error ends only that plugin's matching pending search. Old errors
without an ID remain status messages: background failures must not accidentally
finish an unrelated search. The host deadline handles unresponsive old plugins.

A failed or timed-out search removes that plugin's old results, cancels a queued
activation, and settles once the other plugins finish. A fresh Return can then
use completed results. Duplicate and late responses after completion are ignored.
Actions are matched by both request ID and plugin ID, including error responses;
late action completion after the host deadline does not trigger UI work.

## Search scheduling

Plugins advertise `search.cancel` in `initialized.features`. Only those plugins
receive `cancel_search { query_id }`. Search messages also carry `timeout_ms`;
legacy plugins ignore this additive field. The SDK starts the deadline at receipt,
coalesces adjacent queued searches and preserves execute/configure barriers.
The queue is bounded to 64 messages and 32 MiB; overflow ends the session with an error.

`Plugin::search_with_context` is optional. Existing implementations remain valid;
cooperative implementations check `SearchContext` during expensive work. HTTP,
DNS, Files and File Checksum now cooperate. Child commands drain bounded stdout
and stderr concurrently, and are killed and reaped on cancellation/deadline.
Legacy synchronous implementations cannot be forcibly interrupted inside Rust;
the host still rejects their late results. Background poll deadlines are checked
even when requests continue arriving.

## Typed values and services

`execute.arguments` is a JSON object. The host sends it for declared inline forms
when a plugin advertises `action.arguments`, retaining `input` for compatibility.
Raw external-editor text stays raw. The SDK defaults to adapting arguments into
existing `execute_with_input` implementations.

`initialized.configuration_schema` describes text, integer, boolean and choice
settings, defaults, help text, integer bounds and choices. Nexus supplies a
scrollable Configure form and persists values per plugin. `configure.values`
contains native JSON values when `configuration.values` is supported, alongside
legacy `settings`. SDK validation occurs before calling the plugin; old hosts'
string settings are normalized against the same schema. Schemas are for ordinary
preferences, not credentials. Existing specialized settings pages remain available.

An action may declare `host_action: {service, parameters}`. The object takes
precedence over legacy metadata, and parameters belong to that specific action.
Capability checks still apply. ResultAction source literals in bundled plugins
now specify `host_action: None`; their wire representation is unchanged.

## Negotiation and compatibility fixtures

`initialize` advertises `host_features` and `host_services` (service name to exact
API version). `initialized` returns supported `features`, plus optional
`required_features` and `required_services`. Missing required support rejects the
session before work starts; optional features use the intersection. Missing lists
mean a legacy v1 peer. A newer SDK adapts typed service objects to old metadata for
legacy hosts; conflicting per-action parameters require the modern feature and
fail explicitly rather than selecting the wrong parameters.

The SDK rejects work before initialization and duplicate initialization. The host
validates configuration schemas, including defaults and duplicate field IDs.
`protocol/fixtures/v1.json` is consumed by both Rust and Swift tests. Keep old
fixtures when adding new ones; existing input kinds remain text/integer until a
separately negotiated form extension is introduced.

## Transport and readiness

Both sides cap a newline-delimited JSON message at 16 MiB, including incomplete
frames. The SDK bounds serialization before writing and truncates search results
to the requested limit (at most 100). Host validation also bounds result/action/
field counts. Invalid known events fail the session with a diagnostic; unknown
event types remain ignorable for forward compatibility. Diagnostics name the
plugin and, where parseable, request/query IDs, without echoing message payloads.

The host writes on a dedicated serial queue (64 queued frames, 32 MiB outstanding)
and limits decoded events awaiting main-thread delivery to 128 events and 32 MiB. Pipe writes use
macOS F_SETNOSIGPIPE so plugin exit becomes an I/O failure, not a host crash.
Business requests wait for a validated handshake; startup has a five-second
deadline. Queued pre-initialization searches are coalesced and bounded. Shutdown
terminates the process and releases queues without a main-thread pipe write.

## Plugin status snapshots

The optional `status.snapshots` feature adds a `status_changed` event containing
the complete `presentation_items` array. An empty array explicitly clears a
plugin's status. The SDK sends an initial snapshot after initialization and
subsequent snapshots only when the items change, including after actions and
background polling. It sends no status events to older hosts that do not
advertise the feature. The host keeps snapshots by the current plugin process,
clears them on disable or process exit, and caps each snapshot at 64 items.
Search-bound presentation items remain the fallback for older plugins.

The Plugins page displays the first status item from each reporting plugin, up
to three plugins; selecting a row opens that plugin's own page. Time countdowns
use `countdown_until` and update in the UI without polling the plugin every
second. Reminders reports a pending count and its nearest due time, while its
existing search list holds each reminder's details.

## Validation (2026-09-29)

- Rust workspace: 436 tests passed. Socket-based Clash tests ran outside the
  restricted sandbox against local test listeners.
- Swift: 32 Swift Testing cases plus 5 XCTest cases passed, including real child
  processes for errors, deadlines, stale responses, blocked pipes and readiness.
- Workspace Clippy with `-D warnings`, Rust formatting and whitespace checks pass.
- The final `dist/Nexus.app` is signed with Nexus Development and passes
  `codesign --verify --deep --strict`.
- The independently packaged Hello plugin passes schema/configuration/typed-action
  smoke. Calculator passes old and new handshakes. The native Swift OCR plugin
  passes handshake/search without running clipboard recognition. A local HTTP
  fixture verifies cancellation followed by a responsive new query.

Reproduce the packaged smoke after building the app and Hello example:

```sh
python3 scripts/test-plugin-protocol.py dist/Nexus.app \
  --hello dist/plugins/com.example.nexus.hello.clipkeepplugin --cancellation
```

After explicit restart authorization, the old `/Applications/Nexus.app` process
was stopped and the signed `dist/Nexus.app` launched. Its running executable path
and Nexus Development signature were verified. Desktop checks passed for:

- Calculator search `2+3` displaying `5`.
- Hot-loading the independently signed Hello plugin without restarting the host.
- Return executing Hello's plugin action and displaying its success message.
- Searching Settings and pressing Return to open the settings window.
- The Hello plugin's schema-generated Configure form displaying Greeting/Hello,
  Save and Cancel correctly. The form was cancelled without changing preferences;
  typed configuration application was verified separately by the protocol smoke.

The temporary Hello plugin was removed after testing. After the user authorized
installation, `/Applications/Nexus.app` was replaced through a verified staged
copy, retaining the previous bundle in a temporary backup. All 215 files/symlinks
match `dist/Nexus.app`; deep signature verification passed with Nexus Development.
The app was restarted from `/Applications/Nexus.app`, and its actual running path
and calculator search `2+3` displaying `5` were verified again.
