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
