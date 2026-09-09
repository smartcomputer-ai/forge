# P168 — Environment Skill Catalogs and Catalog Refresh

Status: catalog integration proposed; generic conditional envd scans implemented, 2026-09-09.

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

## Source catalogs and the model-facing catalog

Keep two independently discovered source catalogs in runtime code: VFS skills
and skills from the selected environment. Publish **one effective skill catalog**
to the model after filtering and combining those observations. The source
catalogs are inputs to that projection, not two duplicate menus in context.

VFS locations describe linked workspace/snapshot paths and use `vfs_*` readers.
Environment locations describe environment paths and use ordinary environment
file/process tools. An effective entry includes its identity, name, description,
filesystem domain, skill directory, and readable `SKILL.md` path. Environment
locations also include the environment ID. The selected location's metadata
supplies the description; do not combine one copy's instructions or description
with another copy's directory.

### Which copy is listed

| Available sources | Effective entry |
| --- | --- |
| VFS skill only | List its VFS path. |
| Environment-installed skill only | List its path in the selected environment. |
| VFS skill plus its usable copy under an active environment materialization | List once, preferring the environment path. |
| Environment copy unavailable, missing, or not successfully prepared | Use the accessible VFS entry; expose relevant environment availability separately. |
| Unrelated skills sharing a name | Keep both with distinct source/location labels. |

Prefer an available materialized environment copy for text-only and script-using
skills alike. The model reads instructions and supporting files from one
directory, and can run bundled scripts there. No inspection of the skill body
or heuristic classification of its execution needs is necessary.

Recognize a managed duplicate from the active environment materialization:
source workspace identity and path map to a destination directory and relative
skill path. Require a successfully applied mapping and a discovered, readable
environment `SKILL.md`; the existence of a configured mapping alone is not
enough. Pending source updates use the preparation ordering described above.
Snapshot sources match by snapshot identity and path: a fixed old snapshot
copy must not hide an unrelated live workspace version.

Do not deduplicate by name, description, or matching `SKILL.md` bytes. Supporting
scripts may differ even when the instructions match. Unmanaged copies,
independent CLI installations, and one-off captures/materializations stay
independently discoverable unless they participate in an active registered
mapping. No content comparison or new package-identity registry is needed.

An active mapping establishes the preferred copy, not a promise that its bytes
have never been edited locally. Prefer the current environment file while
that mapping remains valid and usable; ordinary reads return its latest bytes.
Do not add drift detection, body pinning, or activation state for deduplication.
Removing the mapping removes that preference even if its old files remain.

Filter each source by the session's access and discovery configuration before
combining entries. A materialization does not itself grant VFS access or make
an undiscovered environment directory a skill root. Keep alternate authorized
locations and mapping provenance available to API/UI inspection without
advertising duplicate skill entries to the model. A VFS fallback supplies
readable instructions; it does not claim that those scripts can execute in VFS.

### Publication and environment changes

Use one stable context key for the effective menu, for example `skills.catalog`,
and keep source observations/fingerprints in the runtime's discovery records.
Extend the skill catalog envelope and public views to describe mixed locations.
The API's main list should agree with what the model sees, with source details
available for inspection. Its catalog reference names the effective snapshot.

Replace the current VFS-only publication key when implementing this change;
remove its old context entries so both old and new keys do not stay current.
Continue using the existing `SkillCatalog` kind and catalog supersession rules.
Combining sources does not require another engine event or lifecycle.

On an environment switch or deselection, recompute and supersede the effective
catalog before the next model call. A materialized entry can fall back to its
VFS path, and independent environment-only entries leave the current menu.
Previously advertised paths must not be treated as belonging to the newly
selected machine. Historical catalog versions retain their original rendering.

Publish only when the effective menu changes. Scan timestamps, raw source
fingerprints, and transfer receipts belong outside the model-facing payload.
Changes to a preferred path, its environment identity, or useful metadata do
change that payload. If both sources are disabled, clear all versions of the
effective catalog. Preserve existing bounded retention of superseded versions;
do not create a permanent context entry for every environment a session visits.

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
rather than silently merging their contents. Combine managed VFS/environment
copies only through the active-mapping rule above. Multiple usable destinations
for one source use a stable configured mapping-ID/path ordering to select the
preferred location, with the other locations available in inspection views.

Replace the current line-oriented frontmatter parser with YAML parsing that
handles multiline descriptions and nested optional metadata. Share parsing
between VFS and environment catalog builders. Extract the fields needed for
discovery, tolerate unknown fields, and report malformed required metadata per
skill without failing the entire catalog. Do not adopt a vendor's execution
hooks or permission semantics merely because its metadata is present.

## Efficient envd helpers

Use a **bounded filesystem scan** as the reusable environment abstraction.
There is no skill-specific method, frontmatter schema, or catalog type in the
environment protocol. Its current `fs/globFiles`, `fs/searchText`, and
`fs/readFile` operations establish the existing filesystem boundary. Add a
capability-advertised operation that combines matching-file enumeration and
optional content reads in one endpoint-local pass.

Illustrative shape, not final RPC names or DTOs:

```text
fs/scan(
  roots,
  include_patterns,
  read_content,
  digest_algorithm?,
  symlink_policy,
  limits,
  if_none_match?
)
  -> unchanged { fingerprint }
   | result { entries, fingerprint?, complete, diagnostics }

entry:
  root, relative_path, canonical_path, kind
  file: size_bytes, executable, content?, content_digest?
```

For skills the caller supplies patterns matching `SKILL.md` and requests its
bytes. That filename is query data, not a special envd rule. Preserve byte
content with the protocol's ordinary byte representation; UTF-8 decoding and
YAML parsing are caller concerns. The same operation can discover instruction
files, project manifests, configuration files, or template directories with
small metadata files.

Optional per-file digests support reuse planning for other callers without
returning file contents. Start with SHA-256 over complete file bytes, matching
the transfer layer's content identity. Hashing is opt-in; skill discovery only
needs the small matching documents. Include requested digests in the query's
result fingerprint, and bound bytes inspected for hashing separately from
response size. Transfer inventories also include directory entries so empty
directories can be represented.

Reuse the current glob matching conventions. Bound visited entries, depth,
matched files, bytes per file, total bytes, and elapsed time. Return canonical
paths for symlink-alias detection while retaining the requested/root-relative
paths needed to explain locations. Enforce the endpoint's filesystem access
scope and handle loops, missing roots, unreadable files, and symlink target
changes explicitly. Generic environment information such as the execution
user's home directory and default working directory belongs in endpoint
metadata when needed for root resolution, not in a list of skill directories.

Enumerate and read on the machine, returning the bounded result in one response
rather than one network round trip per directory and file. Do not force the
first implementation to maintain a subscription, watch handle, or per-session
scan object. Large transfers remain the separate file/tree transfer facility;
this operation supplies observations, not durable filesystem snapshots.

### Reuse for transfer planning

The [VFS transfer layer](p166-vfs-environment-transfer.md#reuse-and-incremental-transfer)
shares these traversal, metadata, and hashing primitives. An aggregate result
fingerprint answers whether the observed result changed; per-file digests
identify which bytes a receiver can reuse. Materialization already has its
desired inventory in VFS and inspects environment files as reuse candidates.
Capture inventories environment files and uploads only content missing from
the authorized CAS scope.

Keep byte transfer, missing-content negotiation, staging, and publication in
the transfer operations. They require complete selected-tree inventories and
verification that transferred or reused bytes match the advertised digests.
A scan result alone does not pin files for a later read. Large inventories may
use the transfer facility's bounded streaming instead of expanding the small
scan response into a mandatory transfer transport. File reuse preserves full
replacement semantics, including removals; it does not introduce merging.

Provide bounded inventory batching/streaming for the
[large-tree transfer path](p166-vfs-environment-transfer.md#large-trees-and-one-logical-transfer),
sharing traversal and entry semantics with small catalog scans. Operation
completion must establish whether the entire selected inventory was observed;
independent truncated scans cannot be combined into a supposedly complete
tree. Transfer operations own any retained inventory and byte state needed
across requests. This does not require persistent watches or scan handles for
ordinary small per-turn skill discovery.

### Conditional scans and cache correctness

A fingerprint identifies a complete observed result for the same query and
access scope: matching paths, resolved identities, requested metadata, and
requested file bytes or digests. Include additions, deletions, renames, and
symlink retargeting. Content changes invalidate queries requesting bytes or
digests; metadata-only results do not establish byte equality. Exclude volatile
accounting such as scan duration.
An unrecognized fingerprint or a changed query can simply return a full result.

`if_none_match` allows a small unchanged response when the current observation
matches. It saves transport, parsing, and catalog work; it does not imply that
discovering filesystem changes costs nothing. Initially, a bounded local scan
of small matching files is sufficient. Root directory mtime alone is not a
correct change detector, and size/mtime alone can miss in-place file edits.

Later, envd may cache query results and invalidate them with filesystem watches.
Missed/overflowed events, reconnects, or uncertain watcher coverage require a
rescan. A stat or watcher optimization must not turn an unverified stale result
into an unchanged response. These are generic filesystem optimizations, not
skill state, and are not prerequisites for the first implementation.

Only a complete observation may authorize reuse of a previous complete result.
A limit, read error, or detected concurrent mutation must not produce
`unchanged`. Return diagnostics/completeness explicitly and let the runtime
retain its last good observation as stale. Do not silently replace a complete
catalog with a partial listing or treat unavailable roots as confirmed deletions.
Distinguish a successfully observed absent root from failure to inspect it.

Even a complete scan is a live observation, not an atomic multi-file snapshot.
The runtime uses it to build a menu, and subsequent file reads return current
contents. It does not promise that files cannot change between scan and use.

### Runtime composition and reuse

The per-turn path is:

```text
environment preparation for observed workspace revisions
  -> conditional fs scan of configured discovery roots
  -> parse changed SKILL.md metadata in runtime code
  -> combine with VFS discovery and environment materialization records
  -> publish only if the effective menu changed
```

The environment mapping used for deduplication is already stored by Lightspeed:
workspace/source path, environment destination, and application status. Join
those records with observed filesystem locations in the runtime. Envd does
not need a skill identity, a workspace ID, a deduplication rule, or access to
Lightspeed stores. Share traversal/entry primitives with capture where useful
without coupling observation to transfer, storage, or model interpretation.

An unchanged environment scan can reuse its parsed source catalog, but does
not by itself prove that the effective catalog is unchanged. VFS observations,
mapping configuration/application status, selected environment, and source
access can change independently. Recompute that inexpensive composition from
its current inputs and compare the effective result before publishing.

Support a bounded fallback using existing environment filesystem operations
when the helper is unavailable. Missing filesystem capabilities produce an
explicit unavailable source. Partial scans must report their limits and cannot
be passed off as complete catalogs. Keep parsing and composition shared between
the helper and fallback paths. Correctness must not depend on observing writes
through a Lightspeed-specific installer.

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
   and build one effective model/API catalog without activation fields.
2. Implement generic conditional filesystem scans and the capability-negotiated
   fallback. Verify unchanged results, additions/deletions, same-size edits,
   symlink retargeting, changed queries/access scope, and explicit incomplete
   results. Verify optional file digests against content bytes, hashing limits,
   and directory entries used by transfer inventories. No skill types or YAML
   parsing belong in the protocol or envd helper.
3. Consolidate catalog publication and add refresh before model continuations,
   with selected-environment identity and bounded availability handling.
4. Verify direct-install directory layouts, symlink aliases, multiline metadata,
   name collisions, edits/deletions, and filesystem access/scan limits. Verify
   mapped copies appear once at the environment path, unrelated same-name
   skills remain distinct, and unavailable copies fall back to authorized VFS
   paths. Cover mapping removal, partial-subtree mappings, fixed snapshot versus
   live workspace sources, and deterministic preference among multiple copies.
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
- [ ] Effective catalog composition and managed-copy preference implemented.
- [x] Generic envd conditional scans: roots/patterns, metadata or raw content, optional SHA-256, complete fingerprints and partial diagnostics.
- [ ] Catalog fallback for environments without the scan capability.
- [ ] Shared publication and within-run refresh implemented.
- [ ] Installer-layout, availability, cache, and replay checks pass.
