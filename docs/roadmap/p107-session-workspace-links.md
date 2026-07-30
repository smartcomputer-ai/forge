# P107: Session Workspace Links In Config

**Status**

- Implemented 2026-07-30 as a revision to the P62/P95 VFS session-binding model.
- Revised after tracing file tools, VFS projection, prompt instructions, skill
  discovery/activation, Fleet, and Temporal activity boundaries.
- Greenfield change: no compatibility aliases, data migration, dual reads, or
  feature-version transition.

## Decision

Move session VFS bindings into `SessionConfig.features.vfs.workspaceLinks` and
make config events their only durable session store. Keep immutable snapshots,
mutable workspace heads, and CAS data in the VFS catalog, but remove the
separate `vfs_mounts` table and `VfsMountStore` abstraction.

Use **workspace link**, not **mount**, as the product and API term. A workspace
link exposes one catalog resource at a path in the session's workspace
namespace. Its target may be either a mutable VFS workspace or an immutable
snapshot; “workspace” names the session-visible namespace, not only the mutable
target variant.

This follows the declarative MCP split:

- the universe catalog owns the real resource (`mcp_servers`, VFS workspaces,
  snapshots, auth grants);
- session config owns a stable link and its per-session policy; and
- runtime state is derived from config plus the catalog.

`McpServerLink` keeps the server id, optional auth-grant id, and overrides in
config. `WorkspaceLink` likewise keeps a target identity, session path, and
access policy in config. A link to a mutable workspace follows its current head;
advancing that head changes catalog state, not session config revision.

## Config Shape

```jsonc
{
  "features": {
    "vfs": {
      "version": 1,
      "tools": "edit",
      "workspaceLinks": [
        {
          "path": "/workspace",
          "target": {
            "type": "workspace",
            "workspaceId": "workspace_123"
          },
          "access": "readWrite"
        },
        {
          "path": "/skills",
          "target": {
            "type": "snapshot",
            "snapshotRef": "sha256:..."
          },
          "access": "readOnly"
        }
      ]
    }
  }
}
```

The durable vocabulary is `WorkspaceLink { path, target, access }` and
`WorkspaceLinkTarget::{Workspace, Snapshot}`. Do not retain `mountPath`,
`VfsMount*`, or `session/mounts/*` aliases.

Session start and config-put admission validate:

- unique, canonical, absolute, non-overlapping paths;
- existence of every referenced workspace or snapshot at admission time;
- read-only access for snapshot targets; and
- prompt/skill roots against the declared workspace-link paths.

Config stores no catalog revision, workspace head, totals, or availability.

## Durable Declaration And Runtime Resolution

Removing `vfs_mounts` also removes the runtime's current topology lookup by
`session_id`. Do not replace it with session-log replay inside workers. Carry
the event-sourced links explicitly from the workflow's `CoreAgentState` into
the runtime operations that consume them:

- `ToolInvocationBatchRequest` carries workspace links for file-tool routing;
- `RuntimeProjectionRefreshActivityRequest` carries workspace links plus the
  configured prompt and skill sourcing rules;
- gateway operations read links from their already-replayed session state; and
- the in-process runner reads links directly from its `CoreAgentDrive` state.

Temporal activity inputs thereby record the exact session topology used by a
retry. Config remains immutable while a run is active or queued, as it is
today.

Introduce one shared, transient resolver:

```text
WorkspaceLink                         (durable config)
  -> ResolvedWorkspaceLink
       AvailableSnapshot { snapshotRef }
       AvailableWorkspace {
         workspaceId,
         headSnapshotRef,
         workspaceRevision
       }
       Unavailable { reason }
```

The resolver validates topology once and supplies file routing, the VFS
catalog, prompt assembly, skill discovery, and VFS/environment route
composition. Resolved links are never persisted in a replacement table.

Replace `MountedVfsFileSystem` with `LinkedVfsFileSystem`, built from workspace
links and the VFS catalog stores. Remove `VfsMountStore` ownership from
`SessionEnvironmentManager`; callers pass resolved links when composing VFS
and active-environment routes.

## Run-Boundary Projection And Live File Access

Workspace links have two intentionally different resolution cadences:

- **Pre-run derived context** resolves one coherent set of workspace heads and
  uses it for the VFS catalog, prompt instructions, and skill catalog admitted
  before the run.
- **File tools** route through the same declared links but operate on mutable
  workspaces' live heads, including heads advanced by earlier tool calls in the
  run.

Move the VFS-derived pre-run work behind one shared projection path. Gateway,
Temporal, and the in-process runner must use the same resolver and publication
helpers rather than independently interpreting links. The resulting context
commands remain ordinary event-sourced context changes.

The VFS catalog exposes every declared link with path, target, access, and
availability. It is a resolved runtime projection, not another authority for
the declaration.

### Prompt instructions

Prompt discovery keeps the configured/conventional root rules under
`features.vfs.prompts`. For each available workspace link, assembly observes a
specific workspace head and records the head ref and revision in its source
fingerprint. Published instruction bodies remain CAS-backed context entries.

Consequently, advancing or deleting a workspace never mutates instructions
already admitted to an active run. The next pre-run refresh publishes content
from the new head or removes stale instructions from an unavailable link.

### Skills

Skill discovery keeps the configured/conventional root and trust rules under
`features.vfs.skills`. The skill catalog is built from the same pre-run resolved
heads as prompt instructions and records those source identities in its
fingerprint.

Populate `SkillMetadata.skillDocRef` with the CAS ref of the exact `SKILL.md`
body read during catalog construction. Skill activation reads that ref instead
of reopening the mutable workspace through current links. This prevents a
workspace update between discovery and activation from changing the activated
content and keeps cataloged skill content reproducible after source deletion.

## Deletion And Dangling Links

Deleting a VFS workspace succeeds even when session configs reference it, just
as deleting an MCP catalog server succeeds while sessions retain MCP links.
Deletion does not scan or rewrite session logs and emits no config event.

At the next resolution boundary:

- session config still contains the workspace link;
- the VFS catalog reports it as unavailable rather than silently dropping it;
- file operations under its path fail with a structured unavailable-link
  error;
- unrelated links and roots remain usable;
- prompt and skill refreshes attach source warnings and remove stale derived
  entries from the unavailable link; and
- a config put that preserves the dangling link fails admission, while a put
  that removes or replaces it succeeds.

Missing targets are per-link degradation, not whole-session projection
failure. Prompt and skill root resolution must therefore return warnings for
unavailable sources instead of failing before other roots are processed.
Already-admitted CAS context and active skill instructions remain valid until
their normal reconciliation or deactivation boundary.

## API, Runtime, And Storage Changes

- Add `workspaceLinks` to the engine and API VFS feature config.
- Remove `session/mounts/put`, `session/mounts/list`, and
  `session/mounts/delete`; clients edit the full config with its expected
  revision.
- Remove `SessionView.vfsMounts`. Declarations come from session config;
  resolved status comes from the VFS catalog projection.
- Remove `VfsMountRecord`, `VfsMountSource`, `VfsMountTable`, `VfsMountStore`,
  their PostgreSQL/filesystem implementations, and the `vfs_mounts` table. Do
  not add a replacement session-link table.
- Rename public/CAS projection vocabulary from mount to workspace link and
  update the VFS catalog, prompt reports, skill locations, errors, and docs
  consistently.
- Move profile bindings into
  `profile.config.features.vfs.workspaceLinks` and remove the profile-level
  `mounts` section.
- Update CLI `--mount` setup to create the catalog resource and place a
  workspace link in initial session config. The CLI spelling may be renamed
  separately; it is not durable API vocabulary.
- Let clone and fork inherit links through copied config events; remove
  side-table copying. Fleet `share` keeps links unchanged. Fleet `isolate`
  creates child workspaces and replaces child config links before its first
  run; snapshot links remain shared.
- Update prompt/skill live suites, test-support stores, local filesystem
  storage, PostgreSQL tests, the API contract, generated TypeScript client,
  and Configurator MCP facade in the same cut.

## Non-Goals

P107 does not:

- move snapshot manifests, workspace heads, file contents, or CAS ownership
  into the deterministic engine;
- make workspace-link config mutable during active or queued runs;
- pin live file tools to the pre-run head snapshot; or
- add a persistent resolved-link cache. A derived cache may be introduced
  later for performance, but it must not become an authority.

If active-run link mutation becomes necessary later, model it as a separate
event-sourced session component, not as a return to an external link table.

## Done When

- [x] Session workspace links replay entirely from core config events.
- [x] Tool and projection activities receive links explicitly and never load
      session topology from a store by session id.
- [x] No runtime path reads or writes `vfs_mounts` or a replacement link table.
- [x] File routing, VFS catalog projection, prompt instructions, skill
      discovery/activation, and environment composition use the shared link
      resolver.
- [x] One pre-run resolution supplies coherent workspace heads to VFS-derived
      context, while file tools continue to follow live mutable heads.
- [x] Workspace deletion leaves links visible and unavailable, removes stale
      derived content at refresh, and does not block unrelated links.
- [x] Skill activation reads the catalog-pinned CAS document rather than a
      mutable workspace head.
- [x] Profile application, clone/fork, Fleet share/isolate, CLI setup, and the
      in-process runner use config-owned workspace links.
- [x] Unit, integration, and serial Temporal live suites cover replay, workspace
      head changes, deletion degradation, prompt reconciliation, skill
      pinning, and Fleet behavior.
- [x] Current public docs and generated artifacts contain no durable
      session-mount vocabulary; historical roadmap documents retain their
      original terminology.
