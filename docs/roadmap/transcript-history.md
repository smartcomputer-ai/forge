# Progressive transcript history

Status: implemented and verified.

## Behavior

- Open with the latest 500 events, position at the end, and begin independent
  forward long-polling from the initial response's fenced head.
- Automatically fetch older pages near the top, without a load-more button or
  event-count cutoff. A short initial window may load enough to fill the viewport.
- Preserve the visible entry on prepend. New activity follows the bottom only
  while the reader is there; sending and the existing end button resume following.
- Retry older-page failures without stopping live updates. Abort both directions
  and clear the projection on session changes.
- Recover a transient live-read failure with an immediate non-waiting probe
  from the same cursor. Show a disconnect if that probe also fails, pace further
  retries, and clear it on any successful probe, including an empty response.
  Authorization and event-integrity errors stay immediately visible. Bound live
  requests to their requested wait plus ten seconds of transport/projection time.
- Keep the parent/sub-agent header mounted while run activity refreshes its
  children. Background requests and errors retain the current lineage so the
  transcript viewport does not briefly expand and contract on message submission.

## Implementation

`session/events/read` with `direction: "backward"` uses contiguous event sequence
ranges through the existing fork-aware store boundary. It needs no engine replay, checkpoint,
schema migration, or new persisted projection. Each response includes a
chronological window, an exclusive `nextCursor`, `complete`, and `headCursor`.
For backward reads, pass `nextCursor` as `before` until `complete` is true.
Only the initial head initializes forward consumption. Limits bound events,
not bytes: full message text is retained, as in the existing event API.

The browser stores the loaded event window and folds older pages in chronological
order into a fresh display projection. Live lifecycle facts are retained separately
during that rebuild. Stable tool-group identities survive continuation hydration.
Tool results whose starts are outside the window are displayed explicitly as
continued activity. Run usage is shown as a total only after the start event is
loaded. Current control reconciliation includes the dedicated active run, even
when newer queued runs fill the session summary page.

The scroller's native prepend preservation remains enabled. Its sentinel and
loading indicator sit outside the content element to preserve prepend detection.
Loaded history remains in memory for backward reconstruction; this change does
not add DOM virtualization or page eviction. Initial network/projection work is
bounded independently of session length, while reconstruction cost grows with
the history the reader chooses to load.

## Verification

- Server: 12,005 events, concurrent appends, exact backward coverage, empty
  sessions, invalid cursors/combinations, inherited fork prefixes, and a
  non-replayable log proving reads avoid replay. Existing forward wire requests
  remain compatible and the modified Rust consumers compile.
- Browser projection: tool results crossing boundaries, stable group identity,
  lifecycle isolation, duplicate pages, partial usage, and a run over 10,000 events.
- Hook: recent-first loading, concurrent live/history responses, automatic retry,
  cursor gaps, rejection of an old server's forward prefix, empty sessions,
  session switches, and cancellation.
- Reconnection: dropped fetches and gateway errors recover without flashing a
  warning; repeated failures report an outage, idle probes clear it promptly,
  request deadlines recover stalled sockets, and navigation cancels probes/timers.
- Demo: backward cursor parity and concurrent run creation.
- Lineage: delayed run-status refreshes preserve the existing header and links,
  including alongside a parent link; failed refreshes retain the header, and
  switching sessions or universes never shows the previous scope's children.
- Chromium: desktop and mobile tests with 1,200 runs / 12,001 initial events;
  open at the bottom without fetching older pages, preserve the visible offset
  on prepend and concurrent live arrival, reach the first message, and return
  to the latest message. The mobile anchor stayed within one CSS pixel.
- Rust API/CLI tests, runtime library tests, runtime integration-test compilation,
  TypeScript checks/tests, generated-artifact reproducibility, and production/demo
  builds passed. Credentialed/live runtime suites were not run.
