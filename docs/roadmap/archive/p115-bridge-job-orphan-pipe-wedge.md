# P115: Orphaned Descendants Can Wedge A Bridge Job Until Its Timeout

**Status**

- Implemented 2026-08-05. Job and process commands start in their own
  process group; the root process's exit is ground truth for completion.
  On exit the bridge drains remaining pipe output for a bounded grace
  period, sweeps surviving descendants (SIGTERM → grace → SIGKILL), and
  abandons the pipes as a last resort, so nothing a script does to its
  children can hold a job or process open. Cancel and timeout kill the
  whole group. The fact is recorded as `orphanedDescendants` on the host
  `JobSummary`/`ReadProcessResponse`, flows into the model-facing job
  result and `run_process` output note, and is projected through
  `SessionJobSummaryView` in the `api` contract.
- One additional gap found during implementation: `process/start` exit
  observation was driven purely by pipe-reader notifications, so a
  silent child whose descendants held the pipes was only completed by
  its timeout. The bridge now runs a small exit watcher per process that
  polls the child independently of pipe events.
- Deliberately out of scope: PID namespaces (process groups plus the
  drain/abort backstop suffice), killing recorded process groups on
  bridge restart (pid-reuse hazard, no incident demand), and output-size
  caps on job records (separate concern; exposure is bounded by the
  drain grace).
- Covered by host-bridge unit tests: orphan-holding-pipe job completes
  with its real exit code and the orphan flag; a `set -m` escapee
  outside the group still cannot wedge completion; cancel and timeout
  terminate the full group; process terminate and natural exit sweep
  descendants.
- Discovered in production on 2026-08-04 during the first end-to-end
  foundry job on the `ls-dev` environment (`hz02`).

## Incident

A durable environment job (`job_submit`, name `full-run-all-retry`,
1-hour timeout) ran an e2e script that spawned a Temporal worker child.
The script failed and its root bash exited — but the spawned worker
survived the script's SIGTERM teardown (the tsx wrapper died; the real
node process it had exec'd did not) and inherited the job's stdout and
stderr pipes.

The bridge tracks job completion by pipe EOF. With grandchildren holding
the write ends open, the job stayed `running` for ~30 minutes on an idle
machine (load ~0.1) and would have sat there until its timeout. The
session was parked on the job's promise the whole time. An operator
diagnosed the state and killed the orphans by hand; the job then
completed immediately with its real exit code and the run resumed.

## Analysis

Pipe EOF is a proxy for "the job is done" that fails exactly when a job
script mismanages its children — which agent-written scripts will do
routinely. The root process's exit is the ground truth for the job's
outcome; remaining output after it is a bounded courtesy, not a
liveness signal. The client-side workaround (the pack template's e2e
driver now kills its worker's process group with SIGTERM→SIGKILL) fixes
one script; the bridge is the layer that can guarantee it for all of
them.

## Direction (As Shipped)

- Complete the job when the root process exits: reap it, drain remaining
  pipe output for a bounded grace period (seconds), then close the job
  with the real exit code even if descendants still hold the pipes. —
  Done. The exit was already detected (`child.wait()`); completion was
  wedged on the unconditional reader-task joins, which are now bounded
  and backstopped by task abort.
- Start job commands in their own process group (or PID namespace where
  available) and terminate the group on job end, cancel, and timeout, so
  orphans do not outlive the job. — Done with process groups
  (`Command::process_group(0)` plus group signals); PID namespaces were
  rejected as Linux-only and privileged, and liveness never depends on
  catching every descendant.
- Record in the job result whether orphaned descendants were killed —
  that fact is exactly what the submitting agent needs in order to fix
  its script. — Done as `orphanedDescendants` ("detected", since escaped
  descendants are terminated best-effort), surfaced verbatim through the
  normalized job result and as a visible note on `run_process` output.
- Apply the same lifecycle rules to `process/start` sessions on the data
  plane where a disconnected consumer plays the EOF role. — Done: group
  spawn, group kill on terminate/timeout, a descendant sweep when exit
  is observed, reader abort at terminal cleanup, and the independent
  exit watcher described above.
