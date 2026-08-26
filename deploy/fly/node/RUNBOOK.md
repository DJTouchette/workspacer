# Workspacer remote worker node on Fly.io — provisioning runbook

**Written:** 2026-08-24. **Artifacts:** `deploy/fly/node/`.

> **Provisioning both machines? Start at [`../RUNBOOK.md`](../RUNBOOK.md).** This
> document is the reference for *why the node is shaped the way it is*, and its
> prerequisites assume a hub that already exists. The combined runbook is the
> order, and it gathers the steps only a human can do into one sitting.

This is the exact sequence a human runs **once**, in order, to turn an empty Fly
account into a sleeping worker node that the always-on hub can wake. It includes
the three interactive logins that cannot be scripted, and — more importantly —
how to **prove each one survived a stop/start cycle** before anything is built
on top of it.

> The single most important instruction in this document: **do step 7 and step 8
> before you write a line of control-plane code.** If Claude OAuth does not
> survive a stop/start, nothing above it matters.

---

## 0. What you are building

```
   phone / laptop / desktop client
                │
                │ tailnet
                ▼
        ┌───────────────┐   Fly Machines API (public internet)
        │  always-on    │ ──────────────────────────────┐
        │     hub       │                               │
        │  (elsewhere)  │ ◀── brain dials IN over ──┐   │
        └───────────────┘     the tailnet           │   ▼
                                                    │  ┌──────────────────┐
                                                    └──│  Fly worker node │
                                                       │  claudemon (lo)  │
                                                       │  brain --hub …   │
                                                       │  /data volume    │
                                                       └──────────────────┘
```

**Topology is provider-attach, not federation.** The node runs exactly two
workspacer processes: `claudemon serve` bound to loopback, and
`brain --hub <wss url> --token <tok>`, which dials the always-on hub and
registers ~60 capabilities as a provider. There is **no hub on this machine**,
no `peers.json`, no federation link, no `hub:<peer>/` qualification. From the
hub's point of view the node's sessions are ordinary local sessions.

Consequences worth internalising, because they simplify the persistence story:

- **No hub state on the node.** `vapid.json`, `push-subscriptions.json`,
  `jobs.json`, `layout.json`, `peers.json`, `tokens.json` and `remote-token` are
  all *hub-side* files. They matter enormously — on the hub's volume, not this
  one. (`bootstrap.sh` creates their directories here anyway. It is free, and it
  means switching to a federation topology later is a config change rather than
  a volume migration.)
- **Plugins do not load here.** The hub is what loads plugins, and there is no
  hub on this machine.
- **`fleet.quiescence` on the hub already covers this node's sessions**, because
  the node's brain answers `sessions.snapshots` and registers as a Provider
  (which `ClientInfo.UserFacing()` correctly does not count as "someone using
  the machine").

---

## 1. Decisions already made, and why — do not re-litigate these

| Decision | Why |
|---|---|
| **Region `ord` (Chicago)** | Measured 35.4 ms median from Calgary, ahead of `yyz` (~40 ms) with no overlap in the min/max ranges; `sjc`/`lax` tie at 60–63 ms. Chicago is *farther* geographically and still won — Alberta ISPs commonly backhaul through Chicago-area exchanges. **`sea` and `den` no longer exist**: Fly retired them in the September 2025 Region Consolidation Project. Confirmed against `flyctl platform regions`. **The volume pins the region permanently.** |
| **Non-ephemeral, reusable, pre-authorized, tagged auth key** | Tailscale's own Fly guide recommends an *ephemeral* key. That is wrong here. An ephemeral node is removed from the tailnet when it goes offline, and a node that rejoins after removal **gets a new IP** — this machine would churn identity on every wake, which is precisely what persisting `tailscaled.state` exists to prevent. |
| **Kernel-mode tailscaled** | Fly Machines are Firecracker microVMs with a real kernel, so there is no `/dev/net/tun` + `CAP_NET_ADMIN` problem: no `--tun=userspace-networking`, no SOCKS5, no `ALL_PROXY`. Tailscale's own Fly guide invokes the daemon with no `--tun` flag at all. |
| **Debian base, not Alpine** | Kernel mode needs nftables/iptables, and Tailscale warns that on Alpine "you may need to update the machine to ensure that it has kernel support for nftables" — the one realistic way this fails confusingly on Fly. Debian also buys glibc, which Ruby native gems and `rusqlite`'s bundled SQLite both prefer. trixie rather than bookworm because bookworm ships Ruby 3.1 and Rails 8 needs ≥ 3.2. |
| **No suspend** | Not "risky" — **ineligible**. Fly requires ≤ 2 GB memory *and* no swap for suspend. This machine is 8 GB with 4 GB of swap. Setting `auto_stop_machines = "suspend"` would be a no-op or a silent fallback. |
| **`auto_start_machines = true` + `auto_stop_machines = "off"`** | Two independent keys; the combination is legal (verified against flyctl's own config schema) and is exactly the wanted policy: *the proxy may wake it, only the hub may sleep it.* It does not fight the hub-driven design because autostop stays off. It is a token-free backstop for the day the Fly token expires. **But see step 9 — it is gated behind IP allocation on purpose.** |
| **`restart.policy = "on-failure"`, `retries = 3`** | `always` would never let the machine enter `stopped`, defeating the whole design. `on-failure` retries and then leaves it `stopped` — indistinguishable via the Machines API from a healthy sleeping node. That ambiguity is unavoidable at the API; it is made survivable by `/data/state/last-exit.json` (step 8). `retries = 3` rather than the default 10 so a broken boot does not thrash for five minutes burning shared-CPU burst balance. |
| **Non-root (`wks`, uid 10001)** | Claude Code refuses `--dangerously-skip-permissions` when euid is 0, and this fleet leans on full-access grants. Root would be simpler for the volume's file modes; it would break yolo spawns. |

---

## 2. Prerequisites

- `flyctl` installed and authenticated (`fly auth whoami`).
- A Tailscale tailnet with admin access.
- The always-on hub already running somewhere reachable over that tailnet, with
  a known bus URL.
- This repo checked out. **All commands below are run from the repo root.**

---

## 3. Create the app — WITHOUT public IPs

**Do not use `fly launch`.** It allocates public IPs, and this app should not
have one yet.

```sh
fly apps create workspacer-node
```

If you change the app name, change `app =` in `deploy/fly/node/fly.toml` to
match.

> **Why no public IP.** `fly.toml` sets `auto_start_machines = true`, which is a
> genuinely useful token-free wake backstop. But **the Fly proxy starts the
> machine *before* routing the request**, so no application-level auth on the
> doorbell can stop a stranger from spending your money — the wake has already
> happened by the time your code sees anything. The only real control is whether
> the app is publicly routable at all. Step 9 turns it on deliberately.
>
> **The sleep path now exists** (`nodes.sleep`, plus an automatic stop for a wake
> whose provider never registered), so the precondition that gated step 9 is met.
> The doorbell is still off by default and the decision to allocate an IP is still
> a deliberate one — a hub that can turn the machine off does not make a public
> doorbell safe, it only makes it recoverable.

---

## 4. Tailscale: tag first, then the key

The tag is **load-bearing, not cosmetic**. Node keys expire after **180 days by
default**, and a machine that has been asleep for months cannot reauthenticate
itself: it would boot fine, claudemon would start fine, and the node would
simply be unreachable while every Machines API call succeeded. A device that is
**tagged at first authentication has key expiry disabled by default.**

1. In the Tailscale admin console → **Access controls**, add the tag owner and
   an ACL scoping what the node may reach. It needs the hub. If it is running
   agents, it should not have the run of your tailnet:

   ```jsonc
   {
     "tagOwners": {
       "tag:workspacer-node": ["autogroup:admin"],
       "tag:workspacer-hub":  ["autogroup:admin"]
     },
     "acls": [
       // the node reaches the hub's bus, and nothing else
       { "action": "accept", "src": ["tag:workspacer-node"],
         "dst": ["tag:workspacer-hub:7895"] },
       // you reach the node for `fly ssh console`-free debugging
       { "action": "accept", "src": ["autogroup:member"],
         "dst": ["tag:workspacer-node:*"] }
     ]
   }
   ```

2. **Settings → Keys → Generate auth key**, with:
   - **Reusable: yes**
   - **Ephemeral: NO** ← the one that matters
   - **Pre-approved: yes** (if device approval is on for your tailnet)
   - **Tags: `tag:workspacer-node`**
   - Expiry: the maximum, 90 days. It is only used at first boot and at any
     re-auth; an already-connected node is unaffected when it expires.

3. After first boot, **confirm in the admin console that the device shows key
   expiry as disabled.** If it does not, disable it explicitly from the Machines
   page. One click against a failure that would otherwise surface in six months
   with no useful signal.

---

## 5. Mint the hub-side bus token, then set the secrets

On the **always-on hub**, mint a scoped token for the node. The node attaches as
a capability *provider*, so it needs whatever scope your hub requires for that —
if in doubt, the operator tier.

```sh
# on the hub, AS THE USER THAT OWNS ITS CONFIG DIR (on a Fly hub: `su - wks`)
workspacer token create --label fly-node --scope operator
```

The flag is `--label`, not `--name`; `--name` exits 2 with "flag provided but
not defined". No hub restart is needed, because the hub re-reads `tokens.json`
on every new connection.

> **An operator-tier scoped token is TRUSTED on the bus**, exactly like the host
> pairing token, and that includes `nodes.wake`. Verified by calling it: an
> operator-scoped token drove a real `POST /v1/apps/…/machines/…/start`. So this
> credential can spend money, and it lives on the node. It is still the right
> choice, because unlike the host token it can be revoked, but do not read
> "scoped" as "limited" here.

Then, back in this repo:

```sh
fly secrets set --app workspacer-node --stage \
  TAILSCALE_AUTHKEY='tskey-auth-…' \
  HUB_BUS_URL='wss://your-hub.your-tailnet.ts.net/bus' \
  HUB_TOKEN='…'
```

`--stage` holds them until the first deploy rather than triggering one.

`HUB_BUS_URL` is not really a secret; it is set as one to keep your tailnet name
out of git. If the hub's TLS is terminated by `tailscale serve`, remember the
hub itself needs `--trusted-host <magicdns-name>` or every route behind the
proxy answers 403.

---

## 6. First deploy — this creates the volume

```sh
fly deploy \
  --config deploy/fly/node/fly.toml \
  --dockerfile deploy/fly/node/Dockerfile \
  --app workspacer-node \
  .
```

The build context is the repo root deliberately: the image builds `brain` and
`workspacer` from `services/hub` and `claudemon` from `services/claudemon`.

**This deploys the BASE image**, which is workspacer and nothing else — no Go,
Ruby, bun or python3 for application code. That is a working node; it is not a
development box. To put project toolchains on it, build a small image `FROM` this
one and deploy that instead. [BASE_IMAGE.md](BASE_IMAGE.md) is the contract, and
`example.Dockerfile` is a buildable skeleton.

The `[[mounts]] initial_size = "30gb"` creates `wks_data` in `ord` on first
deploy. **Volumes never shrink**, so 30 GB errs high on purpose; they also bill
while the machine is stopped ($0.15/GB/mo = $4.50), are pinned to one physical
host, attach to exactly one machine, and are not reschedulable.

Set snapshot retention deliberately rather than accepting the default —
everything irreplaceable in this design lives on this one volume:

```sh
fly volumes list --app workspacer-node
fly volumes update <vol_id> --snapshot-retention 14
```

### Read the boot log

```sh
fly logs --app workspacer-node
```

You want, in order:

```
=== no previous boot log at /data/logs/prev-boot.log. First boot on this volume, … ===
entrypoint: BOOT <id>
bootstrap: FIRST BOOT on this volume — created 32 directories, layout v1.
bootstrap:   Claude OAuth, git/ssh credentials and the tailnet identity are NOT here yet.
bootstrap: state guard: checking the create-once files
bootstrap:   .credentials.json: absent, and nothing on this volume says it ever existed. First run.
bootstrap:   .claude.json: absent, … First run.
bootstrap:   id_ed25519: absent, … First run.
bootstrap:   tailscaled.state: absent, … First run.
bootstrap:   config.yaml: absent, … First run.
bootstrap:   state.db: absent, … First run.
bootstrap: state guard: no losses detected
entrypoint: doorbell listening on :8080 — wake backstop only
entrypoint: starting tailscaled (state=/data/tailscale/tailscaled.state)
entrypoint: authenticating to the tailnet as 'workspacer-node' (first boot, …)
entrypoint: TAILNET UP after Ns — ipv4=100.x.y.z
entrypoint: running claudemon init (hook port 7890)
entrypoint: claudemon ready (pid …)
entrypoint: starting brain, attaching to wss://… as a capability provider
entrypoint: BOOT COMPLETE <id>
```

**Write down `ipv4=100.x.y.z`.** Step 8 checks it is the same number.

**Those six `absent … First run` lines are the guard working, not a problem.**
None of those files exists yet: the Claude login, the SSH key and the tailnet
identity all arrive in §7, below. The guard records that they were absent, and
from the boot after they appear it will refuse, or warn, if any of them
vanishes. See "The state guard" below.

### Where the boot log lives, and why the first line is a replay

The log is on the volume at `/data/logs/boot.log` (appended, capped at 5 MiB),
`/data/logs/last-boot.log` (this boot only) and `/data/logs/prev-boot.log` (the
boot before this one). **Fly retains logs for 7 days and this machine may sleep
for weeks**, so the platform's logs cannot be the record of why a wake failed.

But a volume is readable only through a shell on the machine it is attached to,
and `fly ssh console` needs a machine that is **running**. This node's restart
policy is `on-failure`: a boot failure retries a few times and leaves the machine
`stopped`. The log written for exactly that situation would sit behind a machine
that will not stay up long enough to read it.

So the entrypoint replays the previous boot to stdout before it does anything
else, and that replay reaches `fly logs`. On the second and every later boot the
log opens like this:

```
=== PREVIOUS BOOT, replayed to stdout so it reaches this run of fly logs. ===
=== Source: /data/logs/prev-boot.log on the volume. This machine cannot be shelled into while it is down. ===
=== verdict: the previous boot COMPLETED. Anything below it is runtime, not startup. ===
=== and its last 40 lines: ===
  | <the previous boot, verbatim>
=== end of previous boot ===
```

Read it in this order:

- **the verdict line** answers the only question that matters, `COMPLETED` or
  `DIED DURING STARTUP`, without you reading anything else;
- **`  ! ` lines**, when present, are every `FATAL` / `STATE LOSS` /
  `REFUSING TO START` / `WARNING` from *anywhere* in that log: a bootstrap
  refusal is near the top, where a tail would never find it;
- **`  | ` lines** are the previous boot quoted verbatim.

**Both prefixes mean "not this boot".** Nothing at the left margin came from an
older log, and the verdict lines deliberately avoid the string `BOOT COMPLETE`,
so `fly logs | grep 'BOOT COMPLETE'` still answers about *this* boot only.

Bounded at 40 lines and 16 KiB (`WKS_BOOT_REPLAY_LINES`, `WKS_BOOT_REPLAY_BYTES`)
so a long log cannot bury the boot you are actually looking at. The replay is
written to stdout only, never back into `boot.log`, otherwise each boot would
quote its predecessor's quote, forever.

### Confirm kernel-mode Tailscale actually came up

Thirty seconds of checking buys certainty about the one inference in the design
that rests on two facts joined rather than a quoted sentence:

```sh
fly ssh console --app workspacer-node -C 'ip link show tailscale0'
fly ssh console --app workspacer-node -C 'ls -l /dev/net/tun'
```

A real `tailscale0` interface means kernel mode. If it is missing, check the
entrypoint's log line about `/dev/net/tun` and the nftables packages.

---

## 7. The three interactive logins

`fly ssh console` lands you as **root**. Become `wks` first — its `.profile`
sources the generated `~/.wks-env`, so the shell gets exactly the environment
the daemons run with:

```sh
fly ssh console --app workspacer-node
su - wks
env | grep -E '^(HOME|XDG_|GOPATH|GOMODCACHE|BUNDLE_PATH|npm_config_cache)='
```

### 7a. Claude Code OAuth — do this one first

This is the credential the state guard refuses to start without once it has seen
it (§10, "The state guard"). Until you do this step it does not exist, the guard
says so and stays quiet; from the boot after it, its disappearance stops the node
rather than letting it come up and hang every session.

```sh
claude
# → /login, follow the URL it prints, paste the code back
```

Then confirm both files exist. **Two files, not one**, and the second is the one
that gets missed:

```sh
ls -l ~/.claude/.credentials.json    # the login
ls -l ~/.claude.json                 # onboarding + the per-project trust map
```

`~/.claude.json` is a **sibling** of `~/.claude`, not inside it. It holds
`hasCompletedOnboarding` and `projects[<cwd>].hasTrustDialogAccepted`. Without
it, the first spawn in each project parks on the interactive folder-trust
dialog — a screen no GUI pane renders and no headless node can answer. The
session simply hangs. This design puts it on the volume because `$HOME` is on
the volume (§10); there is no symlink involved.

### 7b. Git

SSH keys are the supported path:

```sh
ssh-keygen -t ed25519 -C 'workspacer-fly-node' -f ~/.ssh/id_ed25519
cat ~/.ssh/id_ed25519.pub          # add as a deploy key / account key
ssh -T git@github.com              # accepts and records the host key
git config --global user.name  'Your Name'
git config --global user.email 'you@example.com'
```

`gh` is installed if you prefer it (`gh auth login`); its token lands in
`~/.config/gh/hosts.yml`, which is also on the volume.

> Note on `~/.gitconfig`: it is a single file, and file-symlinking it would have
> been a hazard (§10). It is not symlinked here. Measured on git 2.54, `git
> config --global` **dereferences** a symlinked config rather than replacing it —
> but the design does not depend on that, because there is no symlink.

### 7c. Clone your repos

```sh
mkdir -p /data/repos && cd /data/repos
git clone git@github.com:you/your-repo.git
```

`/data/repos` and the agent worktree root (`~/.workspacer/worktrees` →
`/data/home/.workspacer/worktrees`) are **both on the volume, deliberately**.
Git worktrees are two-ended: the checkout holds a `.git` *file* pointing into
`<repo>/.git/worktrees/<name>`, and that admin dir points back. Persist one half
and not the other and you get stale admin entries pointing at dead paths. They
live or die together here.

---

## 8. THE PROOF — stop/start, and check every claim

Nothing above is trustworthy until this passes. Record the "before" values
first.

```sh
# BEFORE
fly ssh console --app workspacer-node -C 'tailscale ip -4'
fly ssh console --app workspacer-node -C 'ls -l /data/home/.claude/.credentials.json /data/home/.claude.json'
fly ssh console --app workspacer-node -C 'sha256sum /data/tailscale/tailscaled.state'

fly machine list --app workspacer-node          # note the machine id
fly machine stop <machine_id> --app workspacer-node
fly machine status <machine_id> --app workspacer-node   # → stopped
fly machine start <machine_id> --app workspacer-node
```

Then walk this checklist. Each row is a distinct failure mode.

| # | Check | Command | Pass |
|---|---|---|---|
| 1 | Tailnet IP is **identical** | `tailscale ip -4` | same 100.x.y.z as before |
| 2 | Tailscale is the *same device* | admin console | one device, not two; key expiry disabled |
| 3 | Claude OAuth survived | `claude -p 'reply with OK'` as `wks` | replies without a login prompt |
| 4 | **Folder trust survived** | clone a repo the node has **never seen**, then spawn an agent in it | no trust dialog, session goes live |
| 5 | Session history survived | `curl -s localhost:7891/sessions \| jq length` | non-zero, and matches pre-stop |
| 6 | Cost accounting works | open a session, check its cost | **non-zero** — see the caveat below |
| 7 | git identity survived | `git config --global user.name` | your name |
| 8 | SSH auth survived | `ssh -T git@github.com` | authenticates, no host-key prompt |
| 9 | Hooks are installed | `grep -c claudemon ~/.claude/settings.json` | non-zero |
| 10 | Boot log has both boots | `tail -50 /data/logs/boot.log` | first boot **and** this one |
| 11 | Later boot took the fast path | `grep 'populated volume' /data/logs/last-boot.log` | present; **no** `FIRST BOOT` line |
| 12 | Exit reason recorded, and **consumed** | `grep 'previous run ended' /data/logs/last-boot.log`, then `ls /data/state/` | the log line carries `"reason":"signal-TERM"`; `last-exit.consumed.json` is present and `last-exit.json` is **gone** |
| 13 | The brain re-registered | hub logs | provider registration, capabilities resolve |
| 14 | **The state guard armed itself** | `ls /data/state/seen` | `claude_.credentials.json`, `claude.json`, `ssh_id_ed25519`, `tailscale_tailscaled.state` |
| 15 | No state loss reported | `grep 'state guard' /data/logs/last-boot.log` | `no losses detected` |
| 16 | **The previous boot reached `fly logs`** | `fly logs --app workspacer-node` | opens with `PREVIOUS BOOT, replayed` and a `verdict:` line |

**Check 14 is what makes checks 3, 4, 7 and 8 stay true.** Those four prove the
credentials survived *this* stop/start. The markers are what makes the *next*
disappearance loud instead of silent: until a file has been seen once, the guard
cannot tell "you have not logged in yet" from "the login was taken away". Four
markers, and if any is missing, the file it names is not there, go back to §7.

**Check 16 costs nothing and is the only one you cannot run later.** It is the
mechanism you will want on the day the node does not come up, and the day it does
not come up is the day you cannot test it.

**Check 4 is the one people skip and the one that bites.** Only a project
directory the node has never seen exercises the trust map in `~/.claude.json`.

**Check 6 is a genuine open question.** Transcripts under `~/.claude/projects`
are the cost ledger — claudemon folds every assistant turn's `usage` block,
including `subagents/*.jsonl` sidechains, to produce both tokens and cost. Its
`path_is_allowed` confinement matches by prefix against `allowed_roots()`. With
`$HOME` on the volume, transcript paths arrive as `/data/home/.claude/projects/…`
and the home root *is* `/data/home`, so this should match — but a refused
transcript path fails **silently**, as zeroed usage and an empty transcript.
Spawn one session and confirm the cost is non-zero before trusting it.

**Check 12 is what makes `stopped` legible.** Fly's `on-failure` policy retries
and then leaves the machine `stopped`, which the Machines API cannot distinguish
from a healthy sleeping node. The entrypoint writes `last-exit.json` on every
exit, so `"reason":"signal-TERM"` (the hub put it to sleep) and
`"reason":"claudemon-died"` (it crashed) are distinguishable from the volume.

**And the second half of check 12 is why you look in the log rather than at the
file.** That record is written by a **trap**, and the exits most worth knowing
about run no trap: a host eviction, an OOM kill of PID 1, a plain `SIGKILL`. Each
ends the machine with nothing written, leaving the record from an *earlier* run
in place, with nothing to date it by. (The entrypoint writes a `bootId`;
`cmd/brain/lastexit.go`'s `exitRecord` has no field for it, so it is dropped at
parse.) A node that slept cleanly, woke, ran for a day and was then hard-killed
would report that old `signal-TERM`, and every client would show a node that went
to sleep on purpose.

So the boot **consumes** the record: it logs it, then renames it to
`last-exit.consumed.json`. A run that was killed without warning now produces
`<no record>`, which is true, instead of the previous run's reason, which is not.
Every reader already treats a missing record as "nobody knows how it ended"
rather than as "it ended cleanly", which is the distinction that makes this safe.

**One consequence, and it is deliberate:** the rename happens before the brain
starts, so `brain.info` no longer carries an exit reason. The record's home is
the **boot log**, which reaches `fly logs` on the next boot without a shell
(§6, "Where the boot log lives"). Putting the `brain.info` half back means moving
the consumption into `cmd/brain/lastexit.go`, read, report, *then* rename, so
that a value which is confidently wrong becomes one that is right and still
travels over the bus. That is a few lines, and it is not done.

---

## 9. Optional: turn on the token-free wake backstop

Only after the hub can put the machine back to sleep. Until then, anyone who
learns the hostname can wake it and it stays awake.

```sh
fly ips list --app workspacer-node      # expect: empty

# EITHER: private (Flycast) — the caller must be on your Fly org's private
# network via `fly wireguard`. No public surface at all.
fly ips allocate-v6 --private --app workspacer-node

# OR: public — <app>.fly.dev becomes a doorbell anyone can ring.
fly ips allocate-v6 --app workspacer-node
fly ips allocate-v4 --shared --app workspacer-node
```

Ring it:

```sh
curl -sS https://workspacer-node.fly.dev/health
```

The proxy holds the request while the machine boots and forwards it once the
internal port accepts. The doorbell answering means **the hardware is up**, not
that the node is ready — readiness is the brain registering with the hub.

To turn it off again: release the IPs, or set `WKS_DOORBELL_ENABLED = "0"` in
`fly.toml` and redeploy (the machine keeps `auto_start_machines`, but nothing
answers, so the request 502s after the boot — which still wakes it; releasing
the IPs is the real off switch).

---

## 10. Persistence: the layout, and why there are no symlinks

### The rule, and the mechanism behind it

**Persist directories, never individual files.** Every atomic writer in this
stack writes a sibling temp file and then `rename(2)`s it over the target.
`rename()` replaces the *directory entry*, so a **file** symlink is destroyed
and replaced by a regular file on the ephemeral rootfs. It works perfectly for
the rest of that boot and the data is gone on the next wake — the exact silent,
intermittent failure this deployment must not have. A **directory** symlink is
safe, because the rename happens *inside* it.

Both halves are demonstrated executably in `test-bootstrap.sh` (section
"SYMLINK RULE — the mechanism it protects against"), so the rule is not folklore.

### How this design solves it: no symlinks at all

Rather than symlink `~/.claude`, `~/.ssh` and friends into `/data` — and then
need special handling for the two that are loose *files*, `~/.claude.json` and
`~/.gitconfig` — **`$HOME` itself lives on the volume** at `/data/home`.
Everything dotfile-shaped is then persistent by construction, with no link for a
`rename()` to destroy. `test-bootstrap.sh` asserts mechanically that zero
symlinks exist anywhere under the volume, so a future edit cannot quietly
reintroduce one.

Two further things fall out of it:

- **The `config.yaml` twins stay in agreement.** It has two writers: the Go half
  reads `$XDG_CONFIG_HOME/workspacer`, the TS half hardcodes
  `~/.config/workspacer`. `XDG_CONFIG_HOME` is pinned to `$HOME/.config` — same
  file. Relocating XDG to a separate `/data/config` would have split them.
- **`$HOME` is pinned explicitly**, both in the environment and in the passwd
  entry. That closes claudemon's third database-path fallback, which is a
  *relative* path under the process CWD — i.e. the rootfs — when `HOME` is unset.
  The database path is also passed explicitly with `--db-path`, at the same
  location the XDG default resolves to, so a hand-run `claudemon` finds the same
  database.

### Where every path from the persistence audit now lives

| Audited path | Verdict | Now at | How |
|---|---|---|---|
| `~/.claude/` (credentials, `projects/`, `settings.json`, `accounts/`) | MUST | `/data/home/.claude/` | `$HOME` on volume |
| **`~/.claude.json`** (sibling, not inside) | MUST | `/data/home/.claude.json` | `$HOME` on volume — **no file symlink** |
| `~/.claudemon/state.db` | MUST | `/data/home/.local/share/claudemon/state.db` | `$HOME` + explicit `--db-path` |
| `~/.config/workspacer/config.yaml` | MUST | `/data/home/.config/workspacer/` | `$HOME` + `XDG_CONFIG_HOME` |
| `~/.config/workspacer/tokens.json` | MUST (hub) | *hub-side*; dir exists here | provider-attach: the node presents `HUB_TOKEN`, it does not host a token store |
| `~/.config/workspacer/remote-token` | MUST (hub) | *hub-side*; dir exists here | ditto — silent re-mint is a hub-side hazard |
| `~/.config/workspacer/peers.json` | MUST (hub) | *hub-side*; dir exists here | no federation on this node |
| `~/.config/workspacer-hub/vapid.json`, `push-subscriptions.json`, `jobs.json`, `layout.json` | MUST/SHOULD (hub) | *hub-side*; dirs exist here | there is no hub on this machine |
| `~/.config/workspacer/claude-profiles.json`, `library/`, `layouts/`, `sessions/`, `logs/`, `plugins/` | SHOULD | `/data/home/.config/workspacer/…` | `$HOME` on volume |
| `~/.workspacer/` (model-rates, scripts, brief.md, handoffs, codex-threads, claude-settings.json) | SHOULD | `/data/home/.workspacer/` | `$HOME` on volume |
| `~/.workspacer/worktrees/` | MUST if repos persist | `/data/home/.workspacer/worktrees/` | same volume as `/data/repos` |
| `~/.ssh/` | MUST | `/data/home/.ssh/` (mode 0700) | `$HOME` on volume |
| `~/.gitconfig` | SHOULD | `/data/home/.gitconfig` | `$HOME` on volume — **no file symlink** |
| `~/.config/git/config` (git's XDG location) | SHOULD | `/data/home/.config/git/config` | `$HOME` + XDG |
| `~/.config/gh/hosts.yml` | SHOULD if you use `gh` | `/data/home/.config/gh/` | `$HOME` on volume |
| `~/.codex/` | MUST if codex is used | `/data/home/.codex/` | `$HOME` on volume |
| `/data/tailscale/tailscaled.state` | MUST | unchanged | explicit `--state` / `--statedir`; the **whole directory**, not just the one file |
| `/data/repos`, Go/bundle/bun/npm caches, shell history | — | unchanged | env vars in `fly.toml [env]` |
| `/data/state/seen/` | bookkeeping | one empty marker per guarded file | the state guard's evidence, "this has existed on this volume". Deleting it disarms the guard |
| `/data/state/last-exit.json` → `last-exit.consumed.json` | bookkeeping | written by a trap at exit, renamed at the next boot once logged | §8 check 12: a record that is never consumed gets re-reported against a run it did not describe |
| `/data/logs/boot.log`, `last-boot.log`, `prev-boot.log` | bookkeeping | all three | `prev-boot.log` is what the next boot replays to `fly logs` (§6) |

### Environment

Set in `fly.toml [env]`, with identical defaults in `entrypoint.sh` so the image
also runs under a plain `docker run`:

```
HOME=/data/home                          XDG_CONFIG_HOME=/data/home/.config
WKS_DATA=/data                           XDG_DATA_HOME=/data/home/.local/share
WKS_HOME=/data/home                      XDG_STATE_HOME=/data/home/.local/state
                                         XDG_CACHE_HOME=/data/cache/xdg
GOPATH=/data/go                          BUNDLE_PATH=/data/bundle
GOMODCACHE=/data/go/pkg/mod              BUN_INSTALL_CACHE_DIR=/data/bun
GOCACHE=/data/go/cache                   npm_config_cache=/data/npm
HISTFILE=/data/home/.bash_history
```

### First boot vs every later boot

`bootstrap.sh` handles the two cases explicitly, because a script that behaves
differently against an empty volume than a populated one is a classic source of
intermittent failure:

| | Empty volume | Populated volume |
|---|---|---|
| directories | created, logged as `FIRST BOOT` | verified; any that went missing are recreated **and reported** |
| `.bashrc` | seeded with the `.wks-env` hook | left alone; the hook is appended once, never twice |
| `~/.wks-env` | generated | **re**generated every boot — it is image-owned, not operator-owned |
| ownership | full `chown -R`, marker written | marker matches → **deep chown skipped** |
| state guard | every guarded file reported `absent … first run`; nothing refused | present files recorded; a file that was recorded and is now **gone** is reported, and a lost Claude credential **refuses** |
| refusal | `/data` missing or not a mountpoint → **exit non-zero**, never runs on the rootfs | same, plus the state guard's own refusal (exit 2) |

The ownership split matters for wake latency. A Fly volume mounts root-owned and
every state file this stack writes is 0600 or 0700, so a uid mismatch is fatal
rather than degraded — but `chown -R` across a 30 GB volume with a populated Go
module cache is minutes against a ~15 s wake budget. The deep pass runs on first
boot, and again whenever the marker is missing or stale (a restored volume, a
rebuilt image with a different uid), and is skipped otherwise.

### The state guard

Everything irreplaceable in this design is on this one volume, and every one of
those files is read by code shaped "read it; if it is not there, carry on". That
is right the first time and wrong every time after. Lose
`~/.claude/.credentials.json` to a snapshot restored from before the login and
this node boots green, registers 67 capabilities, reports `available`, and every
session dispatched to it parks on a login prompt no headless machine can answer.
Nothing errors. Nothing logs. That is the failure this guard exists to end, and
it is the same mechanism the hub uses, deliberately, so the two machines can be
debugged the same way.

The evidence is a marker file under `/data/state/seen/`, one per guarded file,
written the first time that file is seen. "This has existed on this volume
before" is a fact. Absence is only a problem once the marker exists, so a first
boot, where none of these files has been created yet, is quiet.

| File | If it vanishes | Severity |
|---|---|---|
| `~/.claude/.credentials.json` | every dispatched session hangs on a login prompt, silently | **refuse** |
| `~/.claude.json` | the folder-trust dialog nobody can answer; a worker looks alive and does nothing | warn |
| `~/.ssh/id_ed25519` | pushes and ssh clones fail, at least visibly | warn |
| `/data/tailscale/tailscaled.state` | a new tailnet device, a new address, a suffixed name, a duplicate left behind | warn |
| `~/.config/workspacer/config.yaml` | the brain reseeds factory defaults and runs on them | warn |
| `~/.local/share/claudemon/state.db` | every stopped-but-resumable session is gone | note |

**Only the credential refuses, and that is a deliberate asymmetry.** This node's
Fly restart policy is `on-failure`, so a refusal means a few quick retries and
then `stopped`, and `fly ssh console` needs a machine that is running. Refusing
is close to a lockout. It is worth that for the one file whose loss is both total
and completely silent, and not for anything that is either visible when it breaks
or repairable from a shell the node still has.

If it does refuse, the reason reaches `fly logs` on the next start attempt (see
"Where the boot log lives" above), and the way past is one secret:

```sh
fly secrets set WKS_ALLOW_STATE_LOSS=1 --app workspacer-node   # accepts ALL losses, once
# … start the machine, `su - wks`, redo §7a, then:
fly secrets unset WKS_ALLOW_STATE_LOSS --app workspacer-node
```

That is also how you get a shell back on a node that will not boot.

---

## 11. Verify the artifacts locally

Everything here that *can* be checked without deploying, is:

```sh
./deploy/fly/node/test-bootstrap.sh                       # 106 assertions, ~1s, no root/docker/fly
python3 -c "import tomllib;tomllib.load(open('deploy/fly/node/fly.toml','rb'))"
docker run --rm -v "$PWD/deploy/fly/node:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
  -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh verify-image.sh
docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - < deploy/fly/node/Dockerfile
docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - < deploy/fly/node/example.Dockerfile
docker build --check -f deploy/fly/node/Dockerfile .      # BuildKit lint, no image built
```

And the image itself now builds, which is stronger than any of the above:

```sh
# the base — daemons, tailscale, Claude Code, no project toolchains
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .

# a downstream project image, proving the base extends
docker build -f deploy/fly/node/example.Dockerfile \
  --build-arg WKS_BASE=workspacer-node-base:dev \
  -t workspacer-node-example:dev deploy/fly/node

# the 106 assertions again, this time inside the image, as wks, on an empty volume
docker run --rm -v /tmp/fakevol:/data -u 10001:10001 -e HOME=/data/home \
  --entrypoint /bin/bash workspacer-node-base:dev -c /usr/local/lib/wks/test-bootstrap.sh
```

Both builds end in `RUN /usr/local/lib/wks/verify-image.sh`, which asserts the
image contract: nothing installed under a home directory, `/data` empty, no
stateful `ENV`, daemons still on `PATH`, `wks` still 10001:10001, still building
as root. See [BASE_IMAGE.md](BASE_IMAGE.md).

`test-bootstrap.sh` runs `bootstrap.sh` against a temp directory standing in for
`/data` and covers: refusing an unmounted volume, first boot on an empty volume,
every directory from the persistence audit, file modes, the symlink rule (both
that none are created and *why*), a populated volume clobbering nothing across
three boots, a partial volume being repaired **and reported**, a layout-version
upgrade, and the ownership marker's first-boot/later-boot split.

---

## 12. What is NOT verified, because it needs a real machine

Stated plainly so nobody mistakes silence for verification.

1. ~~**The image has never been built.**~~ **It builds.** On 2026-08-24 both the
   base and a downstream project image built clean on amd64: `apt-get`,
   `go build`, `cargo build --release` and
   `npm install -g @anthropic-ai/claude-code` all ran for real. Base is 900 MB.
   `test-bootstrap.sh` passes 63/63 *inside* the image, as `wks`, with an empty
   volume mounted at `/data`, and every binary the node needs resolves under
   those conditions. What that does **not** prove: arm64, a Fly remote builder,
   and `bundle install` against a real Gemfile (no network-bound gem install was
   exercised).
2. **Nothing has been deployed.** No machine, no volume, no `fly.toml` server-side
   validation. The TOML is syntactically valid and every key and enum was checked
   against flyctl v0.4.59's own config struct tags, which is not the same as Fly
   accepting it.
3. **Kernel-mode tailscaled on Fly.** Rests on two facts joined — a Firecracker
   microVM has its own kernel, and Tailscale's Fly guide uses kernel mode — not on
   one quoted sentence. Step 6's `ip link show tailscale0` settles it.
4. **Tailnet IP stability across stop/start.** The whole design assumes it.
   Step 8 check 1.
5. **tailscaled cold-reconnect time.** Neither vendor publishes a figure. The
   entrypoint measures and logs it (`TAILNET UP after Ns`) — this is the single
   most worthwhile day-one number, because no source can tell you.
6. **Cost accounting under a relocated `$HOME`.** Step 8 check 6, and see the
   caveat there: a refused transcript path fails silently as zeroed usage.
7. **Whether a stopped Fly machine severs its TCP connection cleanly.** If it
   goes silent rather than sending RST/FIN, the hub keeps a dead provider
   registered and the woken node's re-registration is **refused** by the router's
   first-registration-wins guard — the machine comes up and provides nothing.
   There is no server-side ping or read deadline on the bus, and the brain never
   subscribes to a topic, so no write ever fails on that connection either. The
   cheap fix is hub-side and doubles as a feature: poll `brain.info` on a timer.
   A `call` writes to the connection, so a dead one fails inside the 5 s write
   timeout and gets dropped — and the same poll is the `available`/`unreachable`
   signal the registry needs anyway. **Flagged for the control-plane worker.**
8. **Whether `fly deploy` starts a currently-stopped machine.** Undocumented.
   A deploy also resets the rootfs. Deploy deliberately; test it once on day one.
9. **Shared-CPU throttling under real builds.** `shared-cpu-4x` gets 20 ms per
   80 ms = 25 % of one core sustained once the 500 s burst balance drains. For a
   box whose job is compiling Go this is the number most likely to make it feel
   broken, and it is easily mistaken for noisy neighbours. Measure steal in
   week one.
10. **Everything in step 8.** The checklist is written; it has not been run.

### Day-one measurements

| Measurement | Why |
|---|---|
| API `start` → brain registered | If under ~15 s the whole caching discussion is moot |
| `TAILNET UP after Ns` from the boot log | The one stage nobody has published a number for |
| Deep-chown duration on first boot | Tells you whether the marker optimisation is load-bearing |
| Warm `go build` on the largest repo | Baseline for whether shared cores are adequate |
| **Spread** across repeated builds, not the average | Wide variance = throttling or neighbours; caching fixes neither |
| CPU steal | Distinguishes your own quota from a noisy neighbour |
| Actual active hours in month one | The cost model assumed 150 |

Costs, current as of 2026-08-24: `shared-cpu-4x`/8 GB is **$0.0617/hr,
$44.44/mo** always-on (the brief's $0.059/$43 was ~4 % low). At 150 active
hours that is $9.26, plus $4.50 for the volume, plus a small rootfs charge that
accrues **while the machine is stopped** — stopped machines are not free.

---

## 13. Assumptions on work owned by other people

These artifacts assume the following land. None of it is implemented here.

1. **`claudemon init` in the boot path.** ~~The entrypoint runs it explicitly,
   because `init` is a *sibling* subcommand of `serve` and nothing in the serve
   path ever invokes it.~~ **UPDATE (as of `9b061244`): `workspacer serve` now
   runs `claudemon init` itself and pins `--db-path` (also new:
   `--claudemon-db-path`, `--no-claudemon-init`, `--allow-new-token`).** That
   fix does not change anything here, though: `entrypoint.sh` drives
   `claudemon` directly rather than going through `workspacer serve` (step 7
   below), so it never gets that pre-flight for free — it still needs its own
   explicit `claudemon init` call and its own explicit `--db-path`, and both
   are correct as written. The workarounds would only become redundant if this
   entrypoint ever switched from driving `claudemon`/`brain` directly to
   shelling out to `workspacer serve` instead.
   On a fresh volume `~/.claude/settings.json` does not exist, so without
   `claudemon init` the hook and statusLine forwarders are absent. The
   symptom is **not** idle sessions — quite the opposite: `internal/quiescence`
   treats `mode: "unknown"` as a blocker, so a hookless session fails safe and
   **pins the machine awake**. The concrete failure is that a PTY session never
   leaves `SessionMode::Unknown`, and a spawn's `first_message` is held until
   the `Input` transition, so **a dispatched PTY worker never receives its
   prompt** — it just sits there looking alive and doing nothing. `claudemon
   init` is idempotent (prints "already up to date" and writes nothing when the
   merge is a no-op).
2. **The git port to the brain.** `git.status`, `git.diff`, `git.log`,
   `git.stage`, `git.commit`, `git.push` — the entire git surface — is a
   *declared* headless gap. Until it lands, this node can run agents but cannot
   show you a diff. Nothing in these artifacts changes when it does.
3. **The Fly control plane — BUILT.** `nodes.list` / `nodes.wake` /
   `nodes.sleep`, the Fly Machines API client, the five-state model, and the
   hub-side stop that puts the machine back to sleep. Both hooks left for it are
   in use: `/data/state/last-exit.json` (which distinguishes a hub-driven sleep
   from a crash loop, as the Machines API cannot) is read on `brain.info`,
   though the entrypoint now consumes the record at boot, so that field reads as
   "no record" until the consumption moves into the brain; see §8 check 12, and
   the doorbell remains deliberately off. Every API stop passes `signal` and
   `timeout` explicitly — `fly.toml`'s `kill_timeout` does not govern a
   hub-issued stop, and `flyapi.Client.Stop` refuses a call that omits either.
   The one thing the entrypoint must keep doing is trapping the signal the hub
   sends (`SIGTERM`) and writing the exit record, because a stop that leaves no
   record is one the next wake cannot tell from a crash.
4. **The hub has its own volume.** Not this machine's problem, but it is the
   hazard most likely to strand the fleet: the hub owns `tokens.json`,
   `remote-token`, `vapid.json` and `push-subscriptions.json`, and on Fly the
   hub's rootfs is rebuilt on every deploy or restart too. A hub restart with no
   volume silently re-mints the remote token and regenerates the VAPID keypair,
   killing every existing web-push subscription with no error anywhere.
