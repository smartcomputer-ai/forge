# Orphaned Descendants Can Wedge A Bridge Job Until Its Timeout

**Status**

- Later / reliability follow-up in `host-bridge` jobs; natural home is
  the P114 bridge workstream (steps 4–6 touch the same code).
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

## Direction

- Complete the job when the root process exits: reap it, drain remaining
  pipe output for a bounded grace period (seconds), then close the job
  with the real exit code even if descendants still hold the pipes.
- Start job commands in their own process group (or PID namespace where
  available) and terminate the group on job end, cancel, and timeout, so
  orphans do not outlive the job.
- Record in the job result whether orphaned descendants were killed —
  that fact is exactly what the submitting agent needs in order to fix
  its script.
- Apply the same lifecycle rules to `process/start` sessions on the data
  plane where a disconnected consumer plays the EOF role.
