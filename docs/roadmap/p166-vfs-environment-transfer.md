# P166 — VFS–Environment Transfer: Materialize and Capture

Status: proposed, 2026-09-08. Design only; not implemented.

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
in the Lightspeed environment configuration,
not in provider-owned machine templates or Incus-specific configuration.

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

## Skill use as an example

A skill saved in a library workspace can be read directly through VFS. An
environment can register that workspace or its skill subtree as a materialization
for initial setup and automatic propagation of later workspace edits. A one-off
transfer can also materialize the selected skill directory when its scripts are
needed. Preserve
the whole directory so relative links to scripts, references, and assets work
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
   staging, operation receipts, and cleanup after failure.
3. Add environment-owned materialization settings/results, automatic workspace
   change propagation, explicit Capture, and creation-time profile defaults.
   Reuse the transfer primitives and existing environment runtime coordination.
4. Connect runtime tools and API operations, workspace source resolution, and
   result retention. Keep all I/O outside the engine. Expose workspace-first
   configuration with replacement as the UI default and update status in
   environment details; keep `error | replace` distinct in the API.
5. Verify binary and text round trips, relative paths, executable scripts,
   symlink handling, target conflicts and full replacement, staging/recovery,
   retries, environment switches, capture changes, workspace conflicts, and CAS
   reachability. Verify replacement removes destination-only entries and leaves
   siblings alone.
6. Verify all workspace commit paths trigger updates, rapid edits coalesce,
   unrelated subtree edits are skipped, missed notifications/restarts converge,
   and offline environments catch up on use without being woken by every save.
   Verify VFS write followed by environment execution waits for the new content.
7. Verify shared-session initialization, overlapping mapping rejection,
   configuration/source revision races, stale retry fencing, removal during a
   transfer, source access, capture coordination, and profile creation defaults.
   Reattachment, profile reapply, and reboot must not replay a completed
   replacement when source contents and configuration are unchanged.
8. Regenerate API and workflow contracts when their wire surfaces change.
   Update the workspace/environment guides and README when the feature ships.

Progress:

- [ ] Transfer contracts and storage ownership settled.
- [ ] Environment helpers and runtime operations implemented.
- [ ] Environment materializations, automatic updates, and edit-then-use ordering implemented.
- [ ] Profile provisioning defaults and environment settings UI implemented.
- [ ] API/tool surfaces and workspace integration implemented.
- [ ] Focused integration, retry, and retention checks pass.
