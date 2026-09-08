# P168 — Environment Skill Catalogs and Catalog Refresh

Status: proposed, 2026-09-08. Design only; not implemented.

Discover skills installed inside the selected environment, including files
written by external installers, and make them available to the model alongside
VFS skills. Add efficient envd filesystem helpers and a shared runtime catalog
refresh path so changes can reach the next model turn without rewriting the
existing prompt prefix.

This builds on [skill simplification](p167-simplify-skill-activation.md) and
preserves [catalog supersession](archive/p136-context-catalogs.md). It does not
depend on [VFS–environment transfer](p166-vfs-environment-transfer.md): discovery works on
directories already present in an environment.

## Behavior

The target interaction is:

```text
model runs an installer in the environment
-> installer writes a skill directory
-> command completes
-> runtime refreshes the environment catalog
-> next model turn sees the skill and reads SKILL.md with environment tools
```

No registration callback from the installer is required. Manually copied
skills, repository checkouts, and materialized directories use the same path.
Editing or removing a skill also becomes visible through discovery.

Environment-owned workspace materializations populate these directories during
initial preparation and automatically after source workspace changes. After
transfer completes, invalidate the environment scan before the next model
request. Order discovery after any pending materializations observed for that
request, using environment readiness coordination; do not publish an old menu
and then replace its files as part of preparing that same request. Discovery
itself does not apply mappings or copy files. Independent installations and
environment-only edits still use ordinary filesystem discovery.

Ordinary re-reads use the latest file contents. Discovery does not pin a skill
body, install dependencies, execute scripts, or create an activation state.

## Catalog sources and locations

Keep VFS and environment discovery independent and present both catalogs to
the model. The VFS catalog continues to describe linked workspace/snapshot
paths and use `vfs_*` readers. An environment catalog describes environment
paths and uses ordinary environment file/process tools.

Extend the source-neutral skill envelope and API list views with environment
source identity and readable locations. A skill needs a stable source-local
identity, name, description, skill directory, and `SKILL.md` path. Environment
locations include the environment ID; a display name alone is not identity.

Use a stable context slot for the currently selected environment's skill
catalog, for example `skills.catalog.environment`, with the environment ID in
the catalog source. Keep `skills.catalog.vfs` independent. The API can present
one combined list while retaining each entry's catalog/source identity; do not
pretend its present single VFS catalog reference describes multiple sources.

On an environment switch, supersede the selected-environment slot before the
next model call. The successor names the new environment and invalidates the
earlier environment menu. On deselection, publish a successor stating that no
environment skills are available, or clear the slot according to existing
feature-removal rules. Previously advertised paths must not be treated as
belonging to the newly selected machine.

If the entire feature is removed, clear all versions of its catalog. Preserve
existing bounded retention of superseded catalogs; do not create a permanent
context entry for every environment a session has visited.

## Discovery roots and interoperability

Configure environment discovery separately from `features.vfs.skills`. Resolve
paths using the environment's execution user and the session's configured
working directory, never the hosted worker's home directory. A temporary `cd`
inside a command does not change the session's discovery scope.

Support project and user `.agents/skills/`, a Lightspeed-specific skill root,
and explicit additional roots. Provide documented compatibility roots for
common client installations, such as `.claude/skills/` and `.codex/skills/`,
without recursively scanning every dot-directory or package cache. Resolve
project ancestry within an explicit repository/work root boundary.

Scan skill directories for `SKILL.md` with bounded depth and file/byte limits.
Follow skill-directory symlinks within the configured filesystem access policy,
including links to an installer's canonical copy. Record canonical directory
identity and deduplicate aliases to the same directory in the same environment.
Detect loops and report unreadable targets. A copied directory with the same
name is not necessarily the same skill.

Keep distinct skills with colliding names addressable by source/location.
Use deterministic ordering and make ambiguity visible to clients and the model
rather than silently merging their contents. A VFS source and an environment
copy remain distinct unless an explicit transfer record establishes provenance.

Replace the current line-oriented frontmatter parser with YAML parsing that
handles multiline descriptions and nested optional metadata. Share parsing
between VFS and environment catalog builders. Extract the fields needed for
discovery, tolerate unknown fields, and report malformed required metadata per
skill without failing the entire catalog. Do not adopt a vendor's execution
hooks or permission semantics merely because its metadata is present.

## Efficient envd helpers

Add a capability-advertised, bounded filesystem scan/read helper to the
environment data protocol and implement it in envd. The request describes
roots, match/depth rules, symlink policy, byte/file/time limits, and optionally
the fingerprint of a previous scan. The response supplies matching paths,
canonical identities, bounded file contents, a fingerprint, and explicit
completion/diagnostics. Exact RPC names and DTOs are implementation work.

Enumerate and read on the machine, returning results in batches rather than
one network round trip per directory and file. A repeated scan may return
unchanged when its observed content fingerprint matches. Root directory mtime
alone is insufficient: editing an existing `SKILL.md` need not change it.

Keep this helper concerned with filesystem observations. Runtime code parses
skill metadata, writes CAS catalog snapshots, and publishes context updates.
Envd does not need to build Lightspeed context entries, query stores, or know
session activation semantics. Factor any shared pure filesystem contracts
without importing hosted runtime implementations into the endpoint.

Support a bounded fallback using existing environment filesystem operations
when the helper is unavailable. Missing filesystem capabilities produce an
explicit unavailable source. Partial scans must report their limits and cannot
be passed off as complete catalogs. Filesystem watches and incremental caches
can optimize later; correctness must not depend on observing writes through a
Lightspeed-specific installer.

Separate scan fingerprints from published catalog content. Body-only changes
can invalidate a scan without changing the model's menu. File mtimes, last-check
timestamps, and cache counters should not cause prompt updates. The model reads
the current body through its normal file tool when needed.

## General catalog refresh

Consolidate the runtime publication path used by VFS, skills, sub-agents, and
future runtime catalogs. Each source keeps its own discovery and freshness
policy; the common path handles stable keys, semantic content comparison,
CAS persistence, publication/removal, and diagnostics. Reuse the existing
supersession mechanism rather than inventing a second catalog event system.

Refresh at a safe boundary before constructing a new model request. This must
include continuations within a run, not only admission of a new idle run. Once
the request is scheduled, its catalog entries and source identities are fixed
for that request and replay; a later observation belongs to a later turn.

For environment skills, check the selected source when it becomes ready and
before subsequent model turns while it is accessible. A command completing,
an environment file mutation, or an observed job completion invalidates local
discovery assumptions. Also perform bounded checks for out-of-band edits; a
cache based solely on Lightspeed tool calls would miss them. A still-running
installer can be observed on a later turn once its writes are complete.

For VFS catalogs, workspace-head changes are natural freshness inputs. Other
catalogs retain their own inexpensive version/fingerprint checks. Reuse one
builder/publication implementation from gateway and workflow paths so an idle
read and a run continuation do not disagree about catalog contents.

External controllers continue to own catalogs they publish through the public
context API. The common runtime refresh path must not overwrite those entries.
VFS-sourced system instructions and frozen tool schemas retain their existing
separate behavior; this proposal generalizes catalog updates only.

Apply context mutations through the existing workflow/engine command boundary.
All environment and storage I/O stays in activities/adapters. The engine keeps
generic catalog ordering, supersession, and retention rules, with no skill-read
or activation state.

## Availability and cache behavior

Do not wake an environment solely because an idle session or skill list is
being inspected. Return the last observation with freshness/availability
information, and refresh after normal readiness handling makes the environment
accessible. A failed scan is distinct from a successful empty scan.

Bound refresh work so a missing or slow environment does not indefinitely
block an otherwise usable session. Keep the last catalog only for that same
source and expose its stale/unavailable status. Never reuse another
environment's menu after selection changes. Diagnostics and observation times
belong primarily in API/debug views; publish a model-facing status update only
when the useful availability state changes.

Unchanged semantic catalogs produce no context update. Changed catalogs append
a successor using current cache-preserving behavior. Earlier entries render
unchanged until normal superseded-entry cleanup. The current catalog survives
compaction as it does today; skill file contents receive no special retention.

## Implementation and verification

1. Add source/location and configuration support, fix shared YAML parsing,
   and expose source-aware list results without activation fields.
2. Implement envd scan/read helpers and the capability-negotiated fallback.
3. Consolidate catalog publication and add refresh before model continuations,
   with selected-environment identity and bounded availability handling.
4. Verify direct-install directory layouts, symlink aliases, multiline metadata,
   name collisions, edits/deletions, and filesystem access/scan limits.
5. Verify an installation completed during a run appears before the next model
   request, an external edit is detected, and an environment switch cannot
   misdirect a refresh or keep the previous menu current.
6. Verify unchanged scans do not append context, body-only edits remain readable
   without menu churn, updates preserve provider prefixes, and normal catalog
   compaction/removal still works. Include replay coverage for changed boundaries.
7. Verify offline/missing capabilities, partial scans, bounded failures, and
   idle reads that do not power on environments. Regenerate affected API/workflow
   contracts and update current skill/environment documentation when shipping.

Use local filesystem fixtures matching installer output for normal tests;
running npm against public registries is not a prerequisite for discovery tests.

Progress:

- [ ] Source-aware catalogs, configuration, and shared parser implemented.
- [ ] Envd helpers and fallback implemented.
- [ ] Shared publication and within-run refresh implemented.
- [ ] Installer-layout, availability, cache, and replay checks pass.
