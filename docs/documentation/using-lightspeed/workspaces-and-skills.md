# Workspaces and skills

A VFS workspace holds persistent files that agents and people can read and
edit. A session links the workspace at an absolute path, such as `/workspace`.
The link makes those files visible to the agent without attaching an operating
system or starting a machine.

The same files can also supply instructions and reusable skills. Prompt files
provide instructions that are loaded for the session. Skills describe
procedures the agent can discover and read when a task calls for them. This
lets a project keep its source material and working conventions together.

This guide extends the `release-notes` workspace from
[Build your first agent](../getting-started/first-agent.md). Use a universe
owner/admin or platform administrator account to manage workspaces and profiles.

## Link files into a session

Create or select a workspace under **Workspaces**. **New file** accepts a path
relative to that workspace, and creates directories in the path as needed.
Open a file, edit its contents, and choose **Save**.

In a profile's **Virtual File System: Files, Instructions, Skills** section,
choose **File tools**, then add a **Workspace link**:

| Setting | Meaning |
| --- | --- |
| **File tools → No file tools** | Supplies no model-callable VFS file operations. |
| **File tools → Read only** | Lets the agent inspect accessible files. |
| **File tools → Edit files** | Adds editing operations, subject to link permissions. |
| **Target type → Workspace** | Reads the live workspace as it changes. |
| **Target type → Snapshot** | Reads an immutable snapshot. Snapshot links are always read-only. |
| **Session path** | The absolute path where this agent sees the linked files. |
| **Access** | Whether this link permits writes to a live workspace. |

To edit `release-notes`, use **Edit files**, a live workspace link at
`/workspace`, and **Read and write** access. A reviewer can use **Read only**
for both the tools and the link. Link paths cannot overlap, and prompt or skill
roots must fall inside a configured link.

A profile linking the same workspace into several sessions shares its live
files. A change made by one session becomes visible to another on a subsequent
file operation. A snapshot gives a reader a fixed version instead. For tasks
that must produce independent artifacts, create separate workspaces or use
different output paths deliberately.

## Add project instructions

In **Workspaces → Release notes → New file**, enter
`.lightspeed/prompts/instructions.md`. Create it, enter this text, and save:

```markdown
# Acorn release documentation

The change list in /workspace/changes.md is the source for release claims.
Use "Acorn 1.2" as the release name. Preserve uncertainty in the source:
an absent compatibility statement is not evidence of compatibility.
```

Open the release-editor profile and set **Prompt roots** to
`/workspace/.lightspeed/prompts`. Save, then create a new session from the
profile or apply the updated setup to an existing idle session.

Lightspeed looks inside each configured root for `instructions.md`, followed
by Markdown files directly inside `instructions.d/` in alphabetical order.
For example, this optional file adds a second instruction source:

```text
.lightspeed/prompts/instructions.d/010-style.md
```

Use it for a short convention such as “Use sentence case for headings.” The
root is explicit: merely placing `.lightspeed` files in a workspace does not
enable sourcing. The UI accepts comma-separated roots, and leaving the field
empty disables it. Roots have a deterministic order; their order in the
textbox is not a priority mechanism.

Sourced files combine with the profile's custom text. Keep them consistent;
their ordering does not resolve conflicting instructions. See
[Profiles and instructions](profiles-and-instructions.md#combine-profile-text-with-workspace-instructions)
for how those sources fit together.

## Add a skill

A skill has its own directory directly inside a configured skill root. Create
`.lightspeed/skills/release-review/SKILL.md` in the same workspace, then save:

```markdown
---
name: release-review
description: Use when checking release notes against a supplied change list.
---

# Review release notes

Read /workspace/changes.md and /workspace/release-notes.md.

For each claim in the release notes, find the supporting change-list entry.
Report unsupported claims, missing changes, and ambiguous source material.
Include enough quoted file text to make each discrepancy easy to locate.

Finish with a short assessment and the changes that need human review.
Ask before editing either file.
```

Set **Skill roots** in the profile to `/workspace/.lightspeed/skills`, keeping
readable file tools and the workspace link enabled. Save the profile and
start a session from it. The resulting workspace layout is:

```text
release-notes/
├── changes.md
├── release-notes.md
└── .lightspeed/
    ├── prompts/
    │   ├── instructions.md
    │   └── instructions.d/
    │       └── 010-style.md       # Optional additional instructions
    └── skills/
        └── release-review/
            └── SKILL.md
```

The filename must be `SKILL.md`. Both `name` and `description` are required
frontmatter fields. Use simple, single-line values as shown; the parser
supports a small frontmatter format rather than the whole YAML language.
Discovery checks skill directories immediately inside each root rather than
recursively searching arbitrary directory trees.

Now send:

```text
Use the release-review skill to check the Acorn release notes.
Read the skill file before reviewing. Report findings without editing files.
```

Inspect the transcript for a read of the skill and both input files. The
catalog initially supplies names, descriptions, and paths; it does not inject
every skill body into the conversation. Reading the relevant procedure keeps
unused skills out of the working context.

## Activate a skill explicitly

The CLI and API can also inject a skill's instructions into an idle session.
This is a separate operation from the model reading `SKILL.md` as a file.
With the [CLI connection settings](sessions-and-runs.md#continue-from-the-cli)
configured, list the discovered skills first:

```bash
target/debug/lightspeed skills list --session "<session-id>"
```

Copy the returned skill ID. It is an identifier from the catalog, not simply
the skill's `name`:

```bash
target/debug/lightspeed skills activate --session "<session-id>" \
  --scope run "<skill-id>"
```

The default `run` scope applies to the next run. Use `--scope session` to keep
the activation across runs, and inspect or remove it with:

```bash
target/debug/lightspeed skills active --session "<session-id>"
target/debug/lightspeed skills deactivate --session "<session-id>" "<skill-id>"
```

The corresponding methods are under `session/skills/` in the
[API reference](../../../crates/api/contract/api-reference.md).

## Update files and handle concurrent edits

Live workspace operations resolve the current workspace revision. Writes
check that revision before committing a replacement. If another writer has
changed it, the write can fail with a conflict; Lightspeed does not merge
competing edits automatically. Reload the current file, compare the changes,
and make the intended edit against the new revision.

Prompt sources and skill catalogs refresh at an idle run boundary, or through
the relevant idle API reads. They do not change the instructions of a run
already executing. After editing an explicitly activated skill, reactivate it
while idle to refresh its injected body. Editing the source file alone does
not replace that active context.

A VFS workspace remains a separate filesystem from any execution environment.
Linking `/workspace` does not mount it into a container or daemon machine.
Transfer files explicitly when a process needs them; see
[Environments](../environments/overview.md).

## If files or instructions are missing

| Symptom | What to check |
| --- | --- |
| A file exists in the browser but the agent cannot find it | Combine its workspace-relative path with the link's session path, and check the current session setup. |
| A write fails despite edit tools | Check link access and whether the target is a snapshot. Read-only links remain read-only. |
| Prompt files have no effect | Configure a nonempty prompt root inside a link, use the conventional filenames, and start the next run after the update. |
| A skill is absent from the catalog | Check the explicit root, direct child directory, exact `SKILL.md` name, and required frontmatter. |
| A discovered skill has not affected the answer | Inspect whether the agent read it, or explicitly activate it for an idle session. Discovery alone loads only its catalog entry. |
| Saving reports a revision conflict | Reload and reconcile with the intervening edit; do not assume the save was merged. |
