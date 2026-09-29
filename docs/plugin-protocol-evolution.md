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
The queue is bounded to 64 messages; overflow ends the session with an error.

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
