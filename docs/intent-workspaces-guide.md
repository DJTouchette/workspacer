# Intent workspaces: preview guide

Intent workspaces keep a feature's desired outcome, execution history, requirements,
review evidence, and decisions together. They work for personal development while
your team continues using its ordinary tickets, Git branches, pull requests, CI,
and reviews. A tracker connection is optional.

This is an opt-in preview. Enable **Settings → Layout → Intent workspaces
(preview)**, then open **Work**. Disabling the setting hides Work and preserves
saved records and existing agent sessions. Enabling the setting does not deploy
or publish anything.

Each saved workspace has **Overview**, **Intent**, **Execution**, **Direction**,
**Review**, **Sources**, **Knowledge**, **Artifacts**, and **History** tabs.
**Overview** brings the saved outcome and work needing attention together; use
the other tabs for the detailed records. The **Projects** button in the sidebar
opens project and repository management.

## Create and execute work

Choose **New intent**, select or enter a project directory on the connected host,
and fill in the title, desired outcome, constraints, and success criteria. Put
each criterion on its own line so you can review it individually. The original
source-link field is a reference; use **Sources** to retain imported requirements.
Choose **Create workspace**. Later changes use **Save revision** and retain the
earlier versions in **History**.

The draft/active/review/complete status describes your work item. It does not
change a ticket's status. Save edits before starting an execution or recording
review decisions; another client's newer revision causes a conflict instead of
overwriting their work. Saved records survive restart. Unsaved drafts are not a
durable substitute for saving.

In **Execution**, choose **Start agent** to use the existing provider/model and
permission dialog. The host records the saved intent revision and exact initial
context packet. **Link agent** associates an existing session without sending it
a message. Branch and pull-request references are tracking links; adding one does
not create a branch or publish a PR.

Open **Context recorded at launch** to inspect what was requested. Editing the
intent later does not silently change that packet or send instructions to an
already-running agent. Retained reports are bounded excerpts of agent messages,
not independently verified results or a complete transcript. Background capture
requires a running owning host; a brief session can disappear between headless
observation cycles.

Future launches, saved directions, and continuation messages include bounded
excerpts of accepted source snapshots, the latest captured version of each
knowledge document, current-revision evidence, and explicit alternative
selections. Source revision/digest and knowledge SHA-256 provenance accompany
the excerpts. Unaccepted source candidates and credential settings are excluded.
Capturing newer context affects newly prepared packets; existing packets keep
their exact original contents.

## Steer and assess delivery

In **Direction**, select a linked execution, record a direction, review the exact
saved message, and explicitly send it. Replacing a direction preserves its
predecessor and receipts. Replacement does not retract queued messages or reverse
work already performed.

In **Execution**, the **Interrupt and continue** controls offer **Interrupt current turn** and
**Continue with a message**. Use **Save control for review**, inspect the saved
request, then **Send interrupt request** or **Send continuation message**.
Interrupt requests an end to the current turn; background work and queued
messages can continue. Continue addresses the same linked session and cannot
restart an ended agent. Neither operation rolls back code or external actions.

**Your assessment → Record assessment** records what you observed and why. An
assessment remains separate from the service's receipt and does not unlock replay
of an uncertain request.

| Receipt | Meaning and recovery |
| --- | --- |
| Accepted | The addressed service accepted the request. Inspect actual agent behavior separately. |
| Failed | The operation was refused or could not be submitted. Correct the stated problem, refresh, and use the explicit retry when offered. |
| Uncertain | The request may have taken effect. Inspect the agent conversation or source ticket. The same operation cannot be replayed automatically or after restart. |

A saved request is not a delivered request. Service acceptance is not proof that
an agent consumed, understood, or applied a direction. A storage failure after
delivery can leave a durable uncertain receipt; refreshing is appropriate,
blindly sending another copy is not.

## Sources and tracker setup

In **Sources**, choose **Source provider**:

- **Manual reference**: enter **Source URL**, **Source title**, and **Source
  snapshot**, then **Import source**. This saves the context you entered without
  contacting the URL. It works without credentials. Record a later manual version
  as another source.
- **Jira Cloud**: use a link such as
  `https://your-site.atlassian.net/browse/TEAM-123`.
- **Azure DevOps**: use a link such as
  `https://dev.azure.com/your-org/your-project/_workitems/edit/123`.

The built-in adapters support those hosted URL forms. Jira Data Center,
on-premises Azure DevOps, alternative API gateways, custom ports, and arbitrary
authenticated endpoints are outside this preview; retain their context with
**Manual reference**. API requests do not follow redirects.

For a tracker, **Host credential environment name** contains an environment
variable's name, never its value. Names must start with `WORKSPACER_SOURCE_` and
contain uppercase letters, digits, or underscores. Set the variable in the
environment of the **owning host process** before starting it. A variable set only
in the browser user's shell does not reach a remote host. For a managed service,
configure its existing launcher and restart that host process.

Jira expects `account-email:API-token`. For example, in a Bash shell that will
launch the host:

```bash
read -r -p 'Jira account email: ' intent_jira_email
read -r -s -p 'Jira API token: ' intent_jira_token
printf '\n'
export WORKSPACER_SOURCE_JIRA="$intent_jira_email:$intent_jira_token"
unset intent_jira_email intent_jira_token
```

Then enter `WORKSPACER_SOURCE_JIRA` in **Host credential environment name**.
Use a token compatible with the site's REST API endpoint, and an account with
permission to view the issue and add comments when publishing.
[Atlassian's authentication guide](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)
describes the email/token format.

Azure expects the personal access token itself:

```bash
read -r -s -p 'Azure DevOps personal access token: ' WORKSPACER_SOURCE_ADO
printf '\n'
export WORKSPACER_SOURCE_ADO
```

Enter `WORKSPACER_SOURCE_ADO` in the form. Work-item read permission is needed for
import/refresh; publishing also needs work-item write permission.
[Microsoft's PAT guide](https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate?view=azure-devops)
and [comment API permissions](https://learn.microsoft.com/en-us/rest/api/azure/devops/wit/comments/add-comment?view=azure-devops-rest-7.1)
describe the supported authentication and access scopes.

Credentials are read on the host at request time. Source records retain the
variable name, provider/native identities, provider-specific fields, source
revision, digest, and fetch time. Imported descriptions are source requirements;
they do not replace your personal intent automatically. Provider transport tests
use simulated responses; a successful live connection depends on your account,
token, and host network configuration.

### Refresh and publish deliberately

Choose **Check source for changes** to fetch again. A change appears beside the
accepted snapshot as **Source changed — review candidate**. Inspect the new
description and **Candidate provider fields**, then use **Accept reviewed source
revision** when appropriate. Earlier accepted snapshots remain available.
Refreshing or accepting never changes ticket status or sends directions to agents.

To publish, select **Comment source**, enter **Comment text**, and choose **Save
comment for review**. Inspect **Review saved comment**, then **Publish reviewed
comment**. Only that saved text is submitted; personal notes are not silently
included. The provider renders the comment using its own format.

Publishing is pinned to the reviewed source and intent revision. A preflight read
refuses observed source drift, and an old proposal must be prepared again after
the source or intent changes. The provider APIs cannot make the revision check
and comment creation atomic: a remote edit can race the final POST. Uncertain
publishing has no resend action; inspect the source's comment history. Confirmed
refusals offer **Retry publishing reviewed comment** when the saved proposal is
still valid.

## Evidence and review

In **Review**, select a **Success criterion**, record **Evidence or verification
notes**, and choose an **Evidence assessment**. The labels distinguish
**Reported**, **Unresolved**, and **Verified by you**. An agent report starts as
something to assess; verification is your explicit claim about the checks you
performed.

For a linked local execution, **Capture Git evidence** retains tracked staged and
unstaged changes against its current HEAD. **Inspect captured diff** opens those
saved bytes. Capture omits untracked file contents and restricted paths, lists
its omissions, and does not run tests, verify a criterion, or archive earlier
commits. Peer execution Git capture is not supported by this host-side operation.
Repository/global Git configuration, content filters, and submodule contents are
excluded; captured working bytes can differ from your configured Git view.

Use **Record my verification** to attach your verification to captured evidence.
Choose **Include in review** on the evidence you are assessing, select **Request
changes** or **Accept reviewed work**, enter a **Review reason**, and choose
**Record review decision**. Acceptance requires selected user-verified evidence
for every current criterion and no selected unresolved evidence. A review is
revision-specific; previous reviews do not verify later intent revisions.
Recording a review does not change the work item's status or publish a team
review. Set the intent's status explicitly when you are ready.

## Artifacts, demonstrations, and alternatives

In **Artifacts and exploration**, upload PNG, JPEG, WebP, GIF, UTF-8 text,
Markdown, or HTML, or enter a URL reference. **Save artifact version** retains
uploaded bytes on the owning host, with a digest and optional criterion/execution
association. Each upload is limited to 512 KiB. URL references retain the URL,
not a snapshot of the remote page.

Choose a **Saved artifact**. **View** displays it; **Annotate** lets you record a
note, and images support a selected point. **Save annotation** binds the note to
that saved artifact version. Uploaded HTML is a static preview: scripts,
navigation, forms, and network resources are disabled. **Open reference** opens a
URL through the ordinary browser path; its live contents are not frozen or
verified by the artifact record.

**Create a demonstration** builds an ordered sequence of saved screenshots with
captions. Select an image and choose **Add selected screenshot as next step**,
write its caption, reorder as needed, and **Save demonstration**. Demonstrations
contain 1–24 steps. This is a screenshot walkthrough, not video recording or
automatic screen capture.

Comparisons hold two to six alternatives, hypotheses, linked artifacts/executions,
and a declared time budget. **Save comparison** retains the exploration; **Record
selection** saves your choice and reason. The budget is not an automatic timer.
Selection does not launch an agent, merge a branch, or apply code. New annotations,
demonstrations, and selections require artifacts/groups from the current intent
revision; older versions remain readable.

## Capture and promote project knowledge

**Project knowledge** lists Markdown documents under this repository's
`.rivet/context/domains`, `modules`, and `paradigms` directories. **Capture document
version** saves the reviewed version, content digest, and capture time. **Captured
knowledge → Read captured version** shows the retained bytes. A later file edit
appears as changed when you refresh the listing.

Under **Reusable finding**, enter **Finding title** and **What should future work
know?**, optionally select supporting captures, then **Record feature finding**.
This creates a feature record without changing project files. No Rivet context is
required to record a finding.

Under **Promote deliberately**, select **Finding to promote** and **Destination**:

- **Rivet learning log · awaiting curation** prepares a new `.rivet/learnings`
  Markdown file.
- **Append to curated project context** prepares an addition to the selected
  existing context document.

Choose **Prepare promotion for review**, inspect **Exact proposed file content**,
then **Write reviewed learning file** or **Apply reviewed context promotion**.
A changed target document or relocated project root refuses an outdated proposal.
There is no automatic promotion and no commit or push. A learning file is a
candidate for curation, not an automatically adopted project rule.

If writing becomes uncertain, **Verify file against proposal** compares the file
with the saved content. A matching file can confirm the write; a mismatch requires
inspection and a newly reviewed proposal. Verification does not repeat the write.

## Projects, storage, and platform limits

Open **Projects** in the sidebar. **Projects and repositories → Manage …** offers **Rename project**, **Add
repository**, and **Relocate repository root**. Work created under an added root
joins that stable project identity. A root cannot belong to two projects.

For relocation, choose **Repository to relocate**, enter **New repository root**
and **Reason for relocation**, then **Save root relocation**. This changes stored
root mappings and adds an intent revision for affected work items. It does not move
files, clone repositories, move running agents, or rewrite fleet configuration.
Move the files yourself or point to an existing checkout. Earlier snapshots and
launch packets keep their historical paths.

State belongs to the connected host. The dedicated `intent-workspaces.sqlite`
database is in that host's Workspacer configuration directory—normally
`~/.config/workspacer` on Linux, `$XDG_CONFIG_HOME/workspacer` when configured,
or `%APPDATA%\\workspacer` on Windows.
Saved artifact/evidence files live beside it in `intent-artifacts` and
`intent-evidence`; project knowledge promotions live in the repository. Preserve
the database and retained-file directories together when backing up or migrating
this preview. Do not treat an unreadable database as an empty workspace.

Retained files and project-knowledge operations support **Linux and Windows**.
Linux pins directory descriptors through `/proc/self/fd`. Windows uses its bundled
Windows PowerShell/.NET helper to open native relative handles and refuse junctions,
reparse points, alternate streams, and unsafe device paths. No extra module or
global installation is needed; Windows PowerShell must remain available under the
host's normal policy. The Windows responsiveness worker is undergoing release verification before
nightly publication.

Other owning-host platforms report secure-file operations as unavailable instead
of weakening path checks. A browser on another platform can use a supported Linux
or Windows host. Ordinary intent records, manual source context, and other
database-only operations do not require this helper.

When a saved file is missing or its digest changes, the preview reports it as
unavailable rather than substituting new bytes. Keep the record for provenance
and save a replacement version when appropriate. For stale-edit errors, refresh
and review the newer state before saving again. For uncertain external operations,
inspect their actual destination before preparing any further action.

The [delivery brief](intent-workspaces.md) records implementation and validation
details. This guide describes the reviewable preview; enabling it remains a user
choice, separate from deployment or live tracker publishing.
