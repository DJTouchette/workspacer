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
today's behavior). The same folder is in **at most one** group; nested folders
may sit in different groups, and the deepest containing folder decides, so
"which project is this session in" always has one answer.

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

- **Projects screen**: a PROJECTS section of cards — mark, name, each folder
  by name and path, running/total sessions, and a "N pending" badge that opens
  Scratch on the project; New agent starts in its first folder; Edit opens the
  editor (name, icon, colour, folders by path or one-click suggestions from
  known folders in no other project). A refused save keeps the draft and says
  why (a stale revision reads "changed elsewhere"); Delete asks twice and
  leaves the folders listed. Below, FOLDERS lists every folder the hub knows,
  each tagged with its project and pending count; pinning applies to folders
  ("Pin folder").
- **Sidebar**: clicking a project filters the sidebar to it (`project_filter`
  holds `group:<id>`): every session in any of its folders, worktree workers
  included. The filter header shows the project's mark, name and pending
  count; the collapsed rail shows the filtered project's mark, which clears
  the filter. The Scratch icon (FolderClosed) carries the total pending count.
- **Scratch page**: opens on the project you are in and lists a multi-folder
  project's scratch a section per folder (`ScratchList {cwds}` →
  `{lists:[…]}`; a mark re-lists them all).
- Reads: `projectGroups.list` and `scratch.pending` are read with the folder
  registry on every (re)connection; `projectGroups.changed` and
  `scratch.changed` re-read them.

Code: `apps/native/src/project_groups.rs` (model), `src/ui/project_groups.rs`
(cards, editor, filter helpers); tests in `src/ui/tests/project_groups.rs` and
`tests/protocol.rs`.

## Not yet

- Desktop (Electron), TUI and mobile list folders only; they ignore groups.
- No MCP tool lists projects for agents yet.
