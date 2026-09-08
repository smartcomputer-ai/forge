# P166 — VFS–Environment Transfer: Materialize and Capture

Status: initial bounded environment endpoint slice implemented, 2026-09-08.
VFS/storage and runtime integration remain proposed.

Provide explicit transfer between Lightspeed's VFS and an
execution environment. The same operations should handle source trees, scripts,
documents, images, audio, and generated outputs. Register reusable workspace
materializations on the environment so its copies follow workspace changes
automatically. Skills are one consumer, not part of the transfer protocol or
storage model.

## Terms and boundaries

Use **VFS–environment transfer** for the feature and these verbs for its directions:

- **Materialize:** write a VFS file or directory snapshot into an environment.
- **Capture:** read an environment file or directory into VFS storage.

These names describe the useful asymmetry: materialization starts from VFS
content; capture creates VFS content from an environment filesystem. CAS is
the underlying byte storage, not a separate artifact-management feature. Avoid
upload/download, whose direction depends on which participant is speaking.
Registered workspace materializations provide automatic one-way propagation;
they do not provide bidirectional synchronization or merge competing edits.

VFS and environment paths remain distinct domains. Transfer has explicit
source and destination identities. It does not mount a workspace into an
environment or route ordinary file reads across domains. One-off transfers
remain copies. Explicitly registered workspace mappings authorize subsequent
automatic copies to their configured destinations; VFS links alone do not.

Related work:

- [Filesystem domains](archive/p113-explicit-vfs-and-environment-tool-domains.md)
  establishes the separation this feature preserves.
- [Skill simplification](p167-simplify-skill-activation.md) makes skill use an
  ordinary file read.
- [Environment skill catalogs](p168-environment-skill-catalogs.md) discovers
  directories regardless of how they arrived in an environment.
- [Shared content](p160-shared-context-content-and-projections.md) supplies the
  existing content descriptors and provenance used by model adapters.
- [Profile provisioning](archive/p125-profile-provisioned-environments.md)
  establishes the pattern of copying initial settings onto an environment.

## Storage model

Reuse CAS for file bytes and VFS snapshots for directory trees. VFS already
records a file's blob reference, size, media type, and executable flag. A
workspace supplies an editable, readable organization of those snapshots;
CAS and workspaces are complementary layers, not competing storage choices.

A source is either a VFS file or a subtree of a VFS directory snapshot.
When a caller supplies a live workspace path, resolve its head once before
starting materialization and record the resolved snapshot. Retries of that
operation use the same bytes. A later transfer can resolve a newer head.

Capture returns a VFS file descriptor or a VFS directory snapshot reference.
The result can be linked into a workspace, attached to context, or used as a
later materialization source. Capturing bytes need not create a workspace.

Record the usual blob containment edges and retain the result through its
durable owner, such as a recorded tool result, workspace, or environment
materialization record. Merely returning a hash must not leave a completed
capture unrooted for CAS collection.

## Operations

The following signatures describe behavior, not final public DTOs or tool names:

```text
materialize(vfs_source, environment_id, destination_path, on_existing)
  -> source_ref, environment_id, destination_path, transferred_totals

capture(environment_id, source_path)
  -> file_or_snapshot_ref, environment_id, source_path, captured_totals
```

Capture environment identity when the operation is scheduled, as ordinary
environment tool calls already do. Changing the session's selected environment
must not redirect an operation or its retry to another machine.

Expose the operations through ordinary runtime tools and public API adapters.
Use bounded environment I/O and CAS references so file bytes do not pass
through model arguments or expand the deterministic workflow history. Longer
transfers can use existing workflow-backed execution facilities; this proposal
does not require a new job or orchestration system.

### Model tools and capability placement

Give the model a pair of explicit transfer tools. Use `capture` consistently
for collecting environment files into VFS; do not add a second synonymous
`collect` tool. Proposed model-facing names and arguments are:

```text
vfs_materialize(source_vfs_path, destination_environment_path,
                on_existing = replace)

vfs_capture(source_environment_path, destination_vfs_path,
            on_existing = replace)
```

The first copies a VFS file or directory into the selected environment. The
second captures an environment file or directory and publishes it at the
selected VFS workspace path. Internally, capture produces immutable stored
content before the revision-checked workspace update; the model does not need
to orchestrate those two storage steps. The low-level API can still return a
standalone capture reference without publishing it into a workspace.

Place these in the VFS tool family and derive availability from the intersection
of existing VFS and environment grants. No new transfer feature toggle is
needed for the initial version:

| Tool | Session grants | Per-call access |
| --- | --- | --- |
| `vfs_materialize` | VFS tools `readOnly` or `edit`, plus environments | Read the VFS source and write the environment destination. |
| `vfs_capture` | VFS tools `edit`, plus environments | Read the environment source and write the VFS workspace destination. |

Environment access alone does not grant VFS access; VFS access alone does not
grant environment I/O. Check linked-route permissions and live environment
capabilities during execution. A snapshot link remains read-only, and a VFS
edit tool grant cannot make it a capture destination. Keep tool definitions
stable when the selected environment changes; ordinary selection/readiness
errors handle a call with no usable environment.

Use the session's existing VFS paths to identify the model's sources and
destinations. Resolve them through its authorized workspace/snapshot links.
Do not expose an arbitrary universe workspace ID or raw blob reference as a
way for a model call to bypass those links. Each selected VFS transfer path
must resolve within one linked workspace or snapshot; copying a synthetic
directory that spans independent links is outside the initial scope.

Unlike ordinary VFS readers/editors, these two operations explicitly depend
on an environment. Classify them as such in dispatch/readiness and tool-batch
validation even though their model-facing names start with `vfs_`. Capture
the selected environment identity when scheduling, prohibit selection changes
in the same dependent batch, and prevent overlapping transfer destinations
from being published concurrently. Ordinary VFS operations remain independent
of environment availability.

Session grants govern these ad hoc model actions. Environment-owned mappings
continue to apply under their own configured source access without requiring
an attached session or enabling its VFS tools. Calling a transfer tool does
not register a mapping or grant the model permission to edit environment setup.

### Partial transfers and replacement scope

Support a single file, a selected directory subtree, or the whole workspace
from the first version. Both directions take an exact source path and an exact
destination path. A directory source's children become the destination's
children; do not silently append the source directory's basename.

| Example | Scope |
| --- | --- |
| Materialize VFS `/library/skills/review` to environment `/opt/skills/review` | Copy only that skill directory and its contents. |
| Materialize VFS `/project/config.json` to environment `/work/config.json` | Copy one file. |
| Capture environment `/work/results/run-42` to VFS `/project/results/run-42` | Replace only that destination subtree; preserve other runs and project files. |
| Materialize a linked workspace root to environment `/work/project` | Copy the whole linked workspace. |

`replace` applies at the selected destination boundary in either direction.
Replacing `/project/results/run-42` in VFS removes destination-only entries
inside `run-42`, while leaving `/project/src`, `/project/results/run-41`, and
other siblings intact. The same rule applies to environment destinations.
Create missing parent directories where permitted; a conflicting non-directory
ancestor is an error. `error` instead rejects an existing selected target.

Publishing a partial capture creates a new workspace head that references the
new subtree and retains all unaffected entries from the observed head. It does
not transfer the entire workspace's file bytes. Reuse existing CAS references
for untouched files and update the head with the normal revision check. A
concurrent commit conflicts, including one outside the replaced subtree;
automatic rebasing or merging is not part of this feature. Keep the captured
content available if workspace publication fails so it can be inspected or
published again without unnecessarily recapturing mutable source files.

The operation is complete or failed for the selected file/tree, not for the
entire workspace. Do not publish half a selected tree on transfer failure.
No include/exclude globs, arbitrary file selection sets, or multi-destination
transactions are needed initially; callers can select a smaller directory or
make separate transfers.

### Materialize

- Preserve relative paths, bytes, and executable flags for the entire selected
  tree. Materializing a directory is useful without interpreting its contents.
- Support `on_existing = error | replace` from the first version. `error`
  requires an absent destination; `replace` replaces the selected target in
  full. Default to `replace` for both one-off transfers and registered
  materializations. Keep the distinction explicit in the API; the normal UI
  uses replacement without an overwrite-policy selector. An API caller that
  chooses `error` receives a conflict when its destination already exists.
- Replacing a directory removes destination-only entries inside that directory.
  It does not overlay source files onto an existing tree or merge file contents.
  Siblings outside the destination are unaffected. The caller selects the whole
  replacement boundary, rather than a set of per-file conflict policies.
- Stage a tree outside the final destination and publish it after all bytes
  have arrived and been checked. A failed transfer must not expose a partial
  final tree as successfully materialized content. Keep an existing target
  intact during staging. Define recoverable replacement at the endpoint;
  do not assume that ordinary rename atomically replaces a nonempty directory
  on every supported filesystem.
- Use an operation identity and completion receipt to recognize successful
  retries. Do not overwrite subsequent local edits merely because an earlier
  successful transfer is retried.
- Return the actual destination path. Consumers use that path for later reads
  and execution; they do not assume that a VFS path is also an environment path.

### Capture

- Read bytes without requiring UTF-8. Preserve supported file metadata and
  relative paths in the resulting VFS content.
- A capture is a bounded observation of a live filesystem, not an atomic
  operating-system snapshot. Detect observable changes during transfer and
  report an incomplete capture rather than claiming a complete stable tree.
  Applications needing a consistent dataset must first make the source stable.
- Publish a successful directory result only after all selected entries have
  been captured. Missing or unsupported entries and exceeded limits are explicit
  failures, not silently omitted files in a supposedly complete snapshot.
- A capture may remain an independent snapshot. Applying it to a live workspace
  is a separate, revision-checked VFS update using the existing conflict rules.
- Record a successful capture result for retry/replay. A new capture request
  reads the environment again and may produce different VFS content.

## Environment materializations

The persistent configuration belongs to the **environment**, not the session.
An environment is a shared, independently lived filesystem: several sessions,
bots, or jobs can use it, and its files outlive any individual attachment.
Session-owned mappings would let different sessions prescribe conflicting
contents for the same directory and make attachment unexpectedly mutate a
shared machine.

Add an environment-owned collection of named materializations. The primary
source is a workspace plus a path within that workspace. Registering it makes
that VFS subtree the source of truth for the destination: saves by humans,
agents, or API clients automatically schedule an updated copy. An immutable
snapshot is an alternative for callers that need a fixed source. Use stable
workspace identity, not a path in one session's VFS link table.

Illustrative configuration, not final public DTOs:

```json
{
  "materializations": [
    {
      "id": "team-skills",
      "source": {
        "type": "workspace",
        "workspaceId": "team-skills-workspace",
        "path": "/"
      },
      "destinationPath": "/opt/lightspeed/skills/team",
      "onExisting": "replace"
    }
  ]
}
```

The snapshot source variant supplies a snapshot reference and source path
instead of a workspace ID. A setting refers to a source; it does not copy the
source bytes into configuration. Retain explicitly configured snapshot refs
and the resolved snapshot needed by a pending or last applied operation through
environment-owned storage roots.

Validate source access and destination paths when configuring and applying
materializations. Source workspaces/snapshots belong to the environment's
universe. Managing these mappings requires environment configuration authority
and access to the source; selecting an environment alone does not grant the
right to change its mappings. Materialized files are then accessible according
to that environment's ordinary filesystem permissions, independently of any
session's VFS links.

Reject overlapping destination paths within the configured collection,
including ancestor/descendant targets. Do not make application order resolve
competing mappings. Serialize materialization operations that touch the same
environment target, and bind each operation to the configuration revision it
was requested for. A late completion must not mark newer settings as applied.

### Automatic application

Saving a mapping or its source workspace is sufficient. There is no required
manual Apply step and no automatic/manual mode selector in the initial UI.
Use replacement by default and show the workspace, source path, destination,
and a small Updating/Current/Error status. Pending work for an offline
environment is shown as waiting for availability. Retain explicit Retry or
Reapply operations for recovery and API use, not as part of the normal edit flow.

| Action | Effect |
| --- | --- |
| Create an environment with materializations | Apply the initial set once the environment filesystem is ready, before releasing dependent workloads. |
| Add or edit mappings on an existing environment | Save configuration and automatically schedule materialization. |
| Edit the source workspace | Automatically schedule its latest committed contents for every environment mapping of the changed subtree. |
| Materialize a snapshot mapping | Use that fixed snapshot; it has no workspace head to follow. |
| Retry the same transfer operation | Reuse its resolved source and receipt; do not resolve a newer head or repeat a completed replacement. |
| Start a session or use the environment | Wait for relevant pending workspace updates before dispatching environment work. |
| Reconnect or wake the environment | Catch up to current source contents; skip already applied, unchanged content. |
| Remove a mapping or change its destination | Leave the old destination on disk; deletion is a separate filesystem action. |

An explicit Reapply is a new operation even when the source snapshot has not
changed: with `replace`, it can restore the target after local edits. Retrying
an already completed operation only returns its recorded result. Normal
workspace updates automatically create new transfer work and require neither.

Track desired configuration/source revision, last applied revision and source
snapshot, destination, and pending/applied/failed result per mapping. Current
means that the latest desired source contents were applied successfully, not
that the environment directory has been continuously checked for local edits.
The initial version does not monitor or repair environment-side drift.

Initial preparation, ongoing updates, and failures belong to environment
lifecycle/runtime coordination shared across sessions. Saving VFS files does
not power on idle environments just to propagate changes. Keep the latest
pending source and catch up through normal readiness handling on next use.
Registered/external environments follow the same behavior when reachable.

### Scheduling and consistency

Observe committed workspace-head changes at the shared store/runtime boundary
so UI saves, agent file tools, direct API updates, and captures all take the
same path. Persist the desired work or recover it by reconciling workspace heads
against applied revisions. An in-memory notification alone is insufficient;
missed notifications and worker restarts must not leave a mapping stale forever.
Keep storage/domain code free of environment network I/O; runtime coordination
consumes the change and runs the low-level transfer.

Coalesce rapid edits and skip a replacement when the selected subtree's bytes,
paths, and executable metadata are unchanged. A workspace edit outside that
subtree does not need to replace its environment copy. Resolve one committed
snapshot per transfer attempt. If newer source contents arrive during staging,
retain newer desired work and converge to it; do not falsely mark the mapping
current when only an older revision completed. Serialize publication and fence
completion by configuration/source revision so an older retry cannot replace
a newer published target. Cancel obsolete work on mapping removal or a change
of destination before publication.

Automatic propagation is asynchronous, but Lightspeed-controlled operations
need a predictable edit-then-use path. Before dispatching environment file,
process, or job work, observe the configured source revisions and wait until
those revisions, or newer ones for the same configuration, are applied. This
includes an agent's VFS write followed by its next environment command. Use a
fixed observed revision for each wait so a continuously edited workspace does
not create an unbounded moving goal. Propagation failures and timeouts are
reported instead of silently running against known stale files.

This gate belongs in environment use/readiness adapters, not in a session-owned
materialization list or a skill activation mechanism. The transfer's own envd
I/O bypasses the consumer gate so waiting for preparation cannot deadlock its
execution. Existing long-running processes and clients accessing the machine
outside Lightspeed are not paused by this gate.

Do not infer a filesystem reset from a new connection or an ordinary reboot.
If a later environment-rebuild feature replaces storage, that operation can
request fresh initialization explicitly. It must not reuse receipts for the
old filesystem. Automatic rebuild detection is not required here.

Automatic replacement affects every user of the environment. The source
workspace is authoritative for the mapped directory. Local changes, including
destination-only files, can disappear on the next source update; put outputs
and scratch work outside mapped input directories. The UI should explain once
beside the mapping that workspace changes replace the environment copy. Do not
introduce merge prompts or a conflict-resolution workflow for local drift.

Replacement does not isolate existing processes or promise that multiple file
reads by an already running job all observe one source revision. Workloads
requiring a fixed tree should use a snapshot source or a separate one-off copy.

### Profiles and reusable setup

Expose the collection directly on environment creation/settings. A profile
that provisions a new environment may supply its initial materializations,
copied onto the created environment just like initial environment credential
bindings. The environment owns the settings and applied state from then on.

Reapplying the profile must not replace an existing environment's mapping
configuration. Its registered workspace sources continue propagating changes
independently of profile application. Profile modes that select an existing
environment or inherit a parent's environment only select it; they do not
apply session-specific workspace overlays. Keep Lightspeed workspace references
in the Lightspeed environment configuration, not in provider-owned machine
templates or Incus-specific configuration.

One-off session/tool transfers remain useful and use the same primitives.
They do not register or mutate persistent environment mappings. This preserves
task-local transfer without making the session the authority for machine setup.

### Capture back to a workspace

Keep capture explicit. A mapping provides convenient defaults for a Capture
action: its environment destination becomes the capture source, and its
workspace source/path becomes the suggested destination. Capture first creates
a snapshot; publishing it back requires the caller's workspace write access
and an expected workspace revision. Serialize a capture with replacement of
the same target while its files are read. A concurrent workspace edit produces
a conflict, not an automatic merge.

Do not write back on session end, environment stop, or disconnect. Snapshot
sources remain immutable; their captures produce a new snapshot or are saved
to a selected workspace. Capture does not reverse or change a mapping by itself.
Publishing captured contents to a mapped workspace is an ordinary workspace
commit and automatically propagates to its mapped environments. Materialization
does not itself write VFS, so this creates no automatic round-trip loop.

## Reuse and incremental transfer

Replacement describes the final selected tree; it does not require sending
every file's bytes on every operation. Include reuse at whole-file granularity
in the initial transfer design. Build a complete inventory, reuse content the
receiver already has, transfer missing content, and publish the complete result.
Deletion and executable-flag changes can change the tree without transferring
any new file bytes. Deduplicating storage after uploading everything would not
provide this transport saving.

Share bounded traversal, path handling, metadata, and optional content hashing
with the [filesystem scan helper](p168-environment-skill-catalogs.md#efficient-envd-helpers).
A transfer inventory describes every selected file and directory, including
empty directories: relative path, entry kind, and, for files, size, executable
flag, and a digest of the complete file bytes. Use SHA-256 over the exact raw
bytes, identical to the existing CAS file-blob digest. Do not include paths,
executable flags, framing, or text normalization in that hash. A protocol
digest represented as `sha256:<lowercase hex>` directly names the corresponding
CAS blob; no rehashing or content-identity translation is needed. Envd does not
need VFS manifests, workspace identities, or storage
access. A scan's aggregate fingerprint identifies its query result and is
separate from a file's content digest or a persisted VFS snapshot reference.

### Materialization reuse

The resolved VFS snapshot already supplies the desired tree and file content
identities; no environment scan is needed to discover the source. The endpoint
uses those identities to find reusable destination files, verifies their bytes,
and reports the missing content. Send only those missing blobs. A persistent
endpoint content cache is optional; verified files in the existing destination
are sufficient to provide reuse for ordinary updates.

The last applied source manifest can identify reuse candidates, but cannot
prove their current bytes: a process may have edited the environment copy.
Verify reusable bytes while constructing the staged result. Use independent
copies or filesystem-supported copy-on-write clones; hard links to writable
destination files would let concurrent edits corrupt staging. Apply executable
metadata separately and omit destination-only entries from the new tree. The
existing replacement and completion guarantees still apply. This can require
local reads and copies of unchanged files even when no network bytes are needed.

Skipping a registered update whose selected VFS subtree has not changed remains
the cheapest path, with the existing rule that ordinary preparation does not
repair local drift. A new transfer or explicit Reapply must verify the content
it reuses; a completed operation retry still returns its receipt.

### Capture reuse

Envd inventories and hashes the selected source locally. The runtime checks
which content is already available in the authorized CAS scope and requests
only missing bytes, deduplicating repeated content within the transfer too.
Build the new snapshot using both reused and newly stored blobs. Retain reused
content through publication so collection cannot invalidate a successful
presence check. A renamed file can reuse its bytes at a new path, and removed
files disappear from the new selected tree. Workspace publication retains its
existing revision and replacement rules even when every blob was reused.

An inventory observation does not pin mutable environment files. Capture must
bind subsequent reads to the advertised digests and verify the received bytes,
or retain verified immutable staging content for the operation. Detected changes
make the selected capture incomplete; do not silently substitute new bytes for
the inventory's content identities. Reuse does not strengthen capture into an
atomic operating-system snapshot.

### Protocol and cost boundary

Use scans for observation and reuse planning. Transfer operations additionally
own missing-content negotiation, bounded byte streaming, verification, staging,
and completion. Share inventory entry types and implementation where useful;
large transfer inventories need not fit in one small `fs/scan` response. A
partial or metadata-only scan cannot establish that the full selected tree's
content is unchanged or authorize deletion of unobserved entries.

Initially, discovering reusable environment content may require reading and
hashing all selected files locally. The savings are in network transfer and
duplicate storage, not a promise of zero filesystem I/O. Hashing limits must
count inspected bytes even when the response returns only digests. A correct
future change-tracking cache can reduce that work. Whole-file reuse does not
require block-level deltas, chunk deduplication, filesystem watches, or a
bidirectional synchronization protocol; a changed large file transfers in full.
Report observed, reused, and transferred totals separately so savings can be
verified without claiming that a reused file was never read.

## Environment support

Put efficient transfer helpers in envd and describe their capability and wire
contract in `environment-protocol`. The existing filesystem methods provide
byte reads and writes, but writes do not carry executable flags, and directory
publication needs an explicit completion operation. Extend those capabilities
or add a bounded tree-transfer helper rather than encoding transfers as
model-generated shell commands.

Use chunking or streaming with limits on file count, bytes, and operation time.
The runtime bridges the environment connection and storage. The protocol should
describe files, metadata, transfer identity, and completion without requiring
an endpoint to access Lightspeed's database or implement its VFS store.

Define symlink behavior before implementing tree round trips. VFS currently
stores files and directories, not symlink entries. The first implementation
should capture a resolved source directory as ordinary files, including when
the selected root itself is a symlink. Internal links may be expanded when
their targets stay inside that resolved tree. Cycles and links outside the
selected tree must be reported; silently dropping them would corrupt a
snapshot. General preservation of symlinks is a separate extension.

Path handling must keep materialized entries within the destination. Support
ordinary files and directories first; report unsupported filesystem objects
and missing metadata capabilities explicitly. These are transfer semantics,
independent of whether a directory happens to contain a skill.

## Relationship to model content

Keep byte transfer separate from rendering or interpreting bytes for a model:

```text
environment file --capture--> VFS file backed by CAS
VFS file bytes --content handling--> provider-supported model input
```

An image already in CAS can reach a provider without an environment or a VFS
workspace. A document conversion may materialize input into an environment,
run a converter, and capture its outputs. The conversion and provider lowering
belong above this layer. OCR, document rendering, transcription, and provider
media capability negotiation are not implemented by VFS–environment transfer.

### Tool output representation and performance

Whole-file transfer reuse does not require changing ordinary tool-result
storage. Current file tools return structured read metadata and line-numbered
model text; process tools return structured output and a presentation containing
status, handles, and other execution information. The runtime persists both
the structured result and the bounded model-facing projection. These formats
serve model behavior, API inspection, and durable replay. A formatted result
containing file text is not the same CAS object as the raw file bytes.

Preserve those model-facing formats and exact historical observations. The
model does not depend on the storage layout behind the rendered result, but
moving formatting from result creation to request assembly would add projection
work and potentially extra blob reads. Such a change must preserve historical
rendering even after files or renderer implementations change.

Do not require an additional raw file or stdout/stderr blob for every ordinary
tool call as part of this proposal. Adding it alongside both existing payloads
can increase hashing, storage operations, retained bytes, and containment-edge
work; deduplication only avoids storing byte-identical objects again. It does
not itself reduce model input tokens or provider request size. Partial reads
and streamed command output must retain their existing bounds, without reading
extra file bytes or aggregating an entire process output merely to obtain a
whole-file hash.

Raw content already held in VFS or by a capture can be referenced without
creating another byte payload, with the usual retention rules. Broader reuse
for complete environment reads is a separate optimization to evaluate against
the current result layout, rather than simply adding a third representation.
Measure tool and prompt-assembly latency, CAS operations, retained bytes, and
actual later-transfer savings before adopting it. Generic shell output remains
an observed stream; do not infer source files by recognizing commands such as
`cat`. Transfer correctness and efficiency must work independently of whether
the model previously read or printed the source.

## Skill use as an example

A skill saved in a library workspace can be read directly through VFS. An
environment can register that workspace or its skill subtree as a materialization
for initial setup and automatic propagation of later workspace edits. A one-off
transfer can also materialize the selected skill directory when its scripts are
needed. Preserve the whole directory so relative links to scripts, references, and assets work
together. Environment skill discovery reads the resulting directory through
its configured roots; transfer does not infer or register skill roots.

A skill installed directly into an environment can be captured into a library
for reuse elsewhere. Discovery and execution do not require that capture to
happen first. Copying a skill does not install its language dependencies,
system packages, external tools, or credentials.

## Implementation and verification

1. Define file/tree inputs, results, limits, collision behavior, and the envd
   capability contract. Reuse existing CAS/VFS representations where possible.
2. Implement bounded capture and materialization, including executable metadata,
   file inventories/digests, missing-content negotiation, whole-file reuse,
   staging, operation receipts, and cleanup after failure.
3. Add environment-owned materialization settings/results, automatic workspace
   change propagation, explicit Capture, and creation-time profile defaults.
   Reuse the transfer primitives and existing environment runtime coordination.
4. Connect runtime tools and API operations, workspace source resolution, and
   result retention. Derive the transfer tool pair from VFS/environment grants
   and validate both filesystem domains during execution. Keep all I/O outside
   the engine. Expose workspace-first
   configuration with replacement as the UI default and update status in
   environment details; keep `error | replace` distinct in the API.
5. Verify binary and text round trips, relative paths, executable scripts,
   symlink handling, target conflicts and full replacement, staging/recovery,
   retries, environment switches, capture changes, workspace conflicts, and CAS
   reachability. Verify replacement removes destination-only entries and leaves
   siblings alone for both partial workspace capture and environment
   materialization, and snapshot links reject capture publication.
   Verify repeated transfers reuse existing content, a one-file edit transfers
   only missing file bytes, and deletions, renames, empty directories, and
   executable-only changes produce the correct tree. Cover environment edits
   invalidating reuse candidates, mutation between inventory and byte reads,
   missing/collected blobs, incomplete inventories, and staging isolation.
6. Verify all workspace commit paths trigger updates, rapid edits coalesce,
   unrelated subtree edits are skipped, missed notifications/restarts converge,
   and offline environments catch up on use without being woken by every save.
   Verify VFS write followed by environment execution waits for the new content.
7. Verify shared-session initialization, overlapping mapping rejection,
   configuration/source revision races, stale retry fencing, removal during a
   transfer, source access, capture coordination, and profile creation defaults.
   Reattachment, profile reapply, and reboot must not replay a completed
   replacement when source contents and configuration are unchanged.
8. Verify tool availability for VFS read/edit with and without environment
   grants, both domains' path access, immutable destinations, environment
   selection races, and partial-capture publication conflicts. A model must
   be able to transfer files through the pair without moving their bytes
   through its own context or constructing storage operations manually.
9. Regenerate API and workflow contracts when their wire surfaces change.
   Update the workspace/environment guides and README when the feature ships.

### Delivered first endpoint slice

`environment-protocol::data::transfer` defines `fs/materialize` and `fs/capture`,
with typed daemon dispatch and `EnvironmentDataClient` methods. These are
standalone environment-side file/tree primitives, not yet VFS operations.
Inputs/results contain raw `ByteChunk` bytes and executable flags, with explicit
root and directory entries (including empty directories). No VFS IDs, storage
access, model tools, API schemas, or workflow history changes are involved.

The initial transport is a **single bounded inline payload**, not streaming or
missing-content negotiation. Mandatory caller limits cannot exceed 1,024 entries
(including the root), depth 32 (root depth zero), 8 MiB per file and in total,
and 30 seconds. Relative inventory paths are at most 4,096 UTF-8 bytes. Zero byte
limits allow empty content; zero entries or duration are invalid. Limit failures
return errors, never partial capture results. Bounds apply to accepted decoded
content; JSON/base64 framing and transport decoding are additional memory costs.
The cooperative deadline is checked during traversal, validation, byte chunks,
and immediately before publication; it cannot interrupt a blocking kernel syscall
or guarantee hard wall-clock completion of failure cleanup.

The implementation requires Linux `openat2`, `renameat2`, and mounted `/proc`.
Other platforms return `Unsupported` and do not advertise transfer capabilities.
Unsupported Linux kernels/filesystems fail explicitly without an unsafe fallback.
Separate capability flags default false for older endpoints; read-only daemons
advertise capture only and reject materialization. Clients must consult the
handshake capability flags before relying on the new operations.

Selected environment paths retain ordinary absolute-host-path / relative-to-cwd
semantics and must fall within the configured filesystem root. Root establishment
rejects symlinks; subsequent traversal uses descriptor-relative no-follow,
beneath-root opens and rejects mount crossings. Open descriptors pin directories
and objects against pathname substitution; external directory renames can change
the pathname of a pinned directory during an operation. This does not isolate
against privileged processes or other processes with access to daemon-private
staging directories. Symlinks at the selected root, ancestors, or inside a capture
are conservatively rejected, including dangling links. Device nodes, sockets,
FIFOs, and non-UTF8 filenames are explicitly unsupported. Symlink expansion as
specified above remains deferred. Regular-file bytes can be arbitrary binary data;
only the executable boolean is preserved (materialized files use 0644 or 0755).

Materialization requires existing destination parents and cannot replace the
configured filesystem root. Inventory parents must precede children; duplicate,
absolute, traversal, and malformed relative entry paths fail validation. Content
is built in a private sibling staging directory before publication. Atomic
`RENAME_NOREPLACE` implements `error`; `RENAME_EXCHANGE` implements replacement
of existing files or nonempty directories, including file/directory transitions.
There is no deletion gap or overlay. Destination-only entries disappear from the
selected target; siblings remain untouched. Pre-publication failures preserve the
old target and clean up staging. Publication is atomic visibility, not crash-durable
storage: this slice does not fsync content or provide crash recovery/receipts.

**Replacement retains the previous complete target** as `tree` in a private
sibling directory returned in `retiredDirectory`. The caller owns its cleanup;
this avoids deleting an arbitrarily large old tree within the transfer deadline.
Repeated replacements without cleanup consume disk. This is an explicit endpoint
resource obligation, not a durable transfer receipt or VFS storage owner. A lost
response can leave an undiscovered retirement directory; automatic reclamation
and retry-safe operation identities remain deferred. No post-publication cleanup
error is misreported as a failed transfer.

Capture reads pinned regular files and directories, checks metadata before/after
reading, and reopens all observed entries from the root for a final metadata
comparison. Observable changes, missing entries, unsupported objects, and exceeded
limits fail the whole response. It remains a live observation, not an atomic
snapshot; callers requiring consistency must stabilize the source.

Focused tests cover binary file/tree round trips, empty directories, executable
flags, read-only capability/dispatch behavior, selected-target collision and full
replacement, sibling preservation, destination-only removal, unsafe paths/links,
special entries, limits, deadline checks, metadata change detection, and a staging
I/O failure that leaves the old tree intact. CAS ownership/retention, VFS subtree
publication, actual concurrent mutation stress, transport interruption/retry
receipts, reuse, streaming, runtime grants, mappings, propagation, profiles, and UI
are not delivered by this slice. Larger progress items below remain unchecked.

Progress:

- [x] Bounded inline environment endpoint primitive and typed client, with focused file/tree tests.

- [ ] Transfer contracts and storage ownership settled.
- [ ] Environment helpers and runtime operations implemented.
- [ ] Whole-file reuse and missing-content transfer verified in both directions.
- [ ] Environment materializations, automatic updates, and edit-then-use ordering implemented.
- [ ] Profile provisioning defaults and environment settings UI implemented.
- [ ] API/tool surfaces and workspace integration implemented.
- [ ] Capability derivation and file/subtree transfer behavior verified.
- [ ] Focused integration, retry, and retention checks pass.
