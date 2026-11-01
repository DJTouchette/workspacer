# Projects: named groups of folders

A **project** is a named, first-class thing that holds one or more folders —
an "Acme" project can be the `api`, `web` and `infra` repositories together.
Sessions, scratch and the native Projects screen are organised by project.

## Additive, beside the per-folder registry

The existing `projects:` map in `config.yaml` stays exactly as it is: keyed by
one folder path, carrying per-folder settings (label, icon, colour, pin,
`lastOpened`, `worktreeSetup`, `git`, `workflowId`, `delivery`, plugins).
Desktop, the TUI, mobile and every hub reader keep using it unchanged.

Groups live in a separate, **hub-owned** file, `<configDir>/project-groups.json`:

```json
{
  "version": 1,
  "revision": 7,
  "groups": {
    "p_3f9c2a1b7d": {
      "name": "Acme",
      "folders": ["/home/me/code/acme-api", "/home/me/code/acme-web"],
      "color": "#5b8def",
      "icon": "🚀",
      "createdAt": "2026-11-01T16:40:00Z",
      "updatedAt": "2026-11-01T16:41:12Z"
    }
  }
}
```

Why not a new key inside `config.yaml`: both config writers (hub `config.save`
and desktop's own TypeScript writer) deep-merge saves, so a client sending back
a stale copy would silently resurrect a deleted group or undo a rename. Only the
`projectGroups.*` methods below write this file, under the config lock and a
revision check; desktop never touches it.

A folder that is in no group is still shown as a one-folder project (exactly
today's behavior). A folder belongs to **at most one** group, so "which project
is this session in" always has one answer.

## Membership

A cwd belongs to the group whose folder contains it (longest folder prefix
wins). A worktree belongs to the group of the repository it was made from
(native maps worktree paths back with `projects::worktree_repo` and session
lineage, as History already does).

## Bus methods

Hub-owned, operator/host only (not view/triage), installed whenever the hub has
a config directory — including the Electron `--hub-only` control plane.

| Method | Params | Result |
|---|---|---|
| `projectGroups.list` | `{}` | `{revision, groups:[{id,name,folders,color,icon,createdAt,updatedAt}]}` sorted by name |
| `projectGroups.save` | `{id?, name, folders, color?, icon?, revision?}` | the saved group plus `revision`. No `id` creates one (`p_` + 10 hex). `revision`, when given, must equal the current one or the save is refused as a conflict. |
| `projectGroups.remove` | `{id, revision?}` | `{removed: true, revision}`; the group's folders, settings and scratch are untouched |

Validation: `name` 1–80 characters after trimming, no control characters;
`folders` 1–32 absolute paths, normalised like `project_key` (forward slashes,
no trailing slash), de-duplicated; a folder already in another group is refused
with that group's name; `color` `#rrggbb`; `icon` at most 8 characters.
Unknown or case-variant keys are refused.

Mutations publish `projectGroups.changed {revision}`.

### Scratch pending counts

| Method | Params | Result |
|---|---|---|
| `scratch.pending` | `{}` | `{projects:[{key, root, pending}]}` for every scratch with a pending file — read from each `index.json` only, no file walks |

`scratch.changed` additionally carries `pending` (that project's pending count
after the change), so clients can update a badge without a re-read.

## Native

- **Projects screen**: projects (groups) first, each a card with its mark, name,
  folders, live/ended session counts and a pending-scratch badge; then folders
  that are in no project. New project / Edit opens an editor (name, icon,
  colour, folders: add from known folders, typed path or the hub folder
  browser; remove). Delete asks twice.
- **Sidebar**: the project filter can be a whole project (every session in any
  of its folders, worktrees included). The Scratch rail/footer icon carries the
  total pending count; the filter header carries the filtered project's.
- **Scratch page**: picks a project; a multi-folder project lists each folder's
  scratch in its own section.
