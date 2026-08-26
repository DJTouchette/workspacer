# Provisioning the Fly fleet: the ordered runbook

**Written 2026-08-25, after a full dry run.** Two machines, in one order, with
the parts only a human can do gathered into one sitting.

There are already two runbooks here, one per machine:
[`hub/RUNBOOK.md`](hub/RUNBOOK.md) and [`node/RUNBOOK.md`](node/RUNBOOK.md).
They are the reference for *why* each machine is shaped the way it is, and this
document does not repeat them. What they do not give you is the **order**, and
the order matters, because each machine's runbook lists the other machine as a
prerequisite. Follow either one alone and you stop halfway.

This document is the sequence. It also states honestly which steps have been
rehearsed and which have not, because a runbook nobody has walked is worth
very little.

---

## The shape of it

| Part | What | Needs an account? | Your hands-on time |
|---|---|---|---|
| **A. Pre-flight** | build and verify both images | no | one command |
| **B. The block** | the three human gates, and the deploys between them | yes | about 15 minutes |
| **C. The proofs** | stop/start, restart, and the checklists | yes | 20 minutes, later |

Part A needs no Fly account, no tailnet, no credential and no money. Do it a
day early if you like. When it is green, everything that can be proved without
spending has been proved.

**Read this before you plan the evening.** Part B contains two long waits you
cannot shorten: the two `fly deploy` uploads. The images are large (the base is
about 900 MB, the hub about 920 MB, and a project image with toolchains on top
is around 2.3 GB), and `--local-only` means they are pushed from your machine to
Fly's registry over your own uplink. Your hands-on time really is about fifteen
minutes. The **wall clock** depends entirely on your upload speed, and nobody
has measured it on a real deploy. Plan for the uploads to be the longest part of
the evening and start them before you need the machine.

---

## Why the order is what it is

The two machines depend on each other, in opposite directions, at different
moments:

- The **node** cannot be configured until the **hub** exists, because the node's
  `HUB_BUS_URL` secret is the hub's MagicDNS name, and its `HUB_TOKEN` is minted
  on the hub.
- The **hub** cannot finish until the **node** exists, because `nodes.json`
  needs the node's Fly machine id, and a machine id only exists after a deploy.

So it is not hub-then-node. It is **hub, node, then back to the hub once.** The
last step is a two-minute edit and a restart, and if you do not know it is
coming you will think you are finished when you are not.

```
  Tailscale keys ─┐
  Fly spend ──────┤
                  ▼
             hub app + secrets + deploy ──► MagicDNS name, HUB_TOKEN
                  │
                  ├─► mint the node's bus token (on the hub)
                  ▼
             node app + secrets + deploy ──► machine id
                  │
                  ├─► back to the hub: nodes.json, restart
                  ▼
             Claude OAuth on the node
```

---

## Write these down as you go

Six values travel between steps. Copy this block somewhere before you start and
fill it in as each one appears, so you never have to scroll back.

```
TAILSCALE_AUTHKEY (hub)   tskey-auth-........................
TAILSCALE_AUTHKEY (node)  tskey-auth-........................
FLY_API_TOKEN             FlyV1 fm2_.........................
HUB MagicDNS name         workspacer-hub.<your-tailnet>.ts.net
HUB_TOKEN                 ................................ (32 chars)
NODE machine id           ..............
```

---

# Part A. Pre-flight

No account, no credentials, no cost. One command:

```sh
./deploy/fly/preflight.sh
```

It rebuilds both images, checks both `fly.toml` files against the decisions
they encode, runs 63 + 111 bootstrap assertions on the host and again inside the
image as `wks` on an empty volume, runs shellcheck and hadolint and
`docker build --check`, runs the base image's own `verify-image.sh` contract
check, and then does the thing none of those do: it **boots both entrypoints for
real** and requires each to reach `BOOT COMPLETE`.

That last stage exists because this repo shipped a node image that could not
boot while every other check was green. `entrypoint.sh` called
`setpriv --inherit-caps=-all`, which util-linux does not have (it is
`--inh-caps`), so setpriv exited before running anything. A full build, the
bootstrap suite, hadolint, shellcheck and an empty-volume smoke test all passed
over it, because tailscaled cannot come up on a build host and the entrypoint
never reached that line. **A build proves assembly. It does not prove boot.**
The rehearsal stage replaces exactly two binaries, `tailscale` and `tailscaled`,
and runs everything else verbatim.

### Do not skip the rebuild

`preflight.sh` rebuilds even when nothing looks like it changed, and that is
deliberate. `workspacer-node-base:dev` is a **mutable local tag**, and the hub's
Dockerfile says `FROM workspacer-node-base:dev`. On 2026-08-25 the base image on the
machine this was rehearsed on had been built 38 minutes before the setpriv fix
landed, and still carried the dead line. Nothing about looking at the tag would have told
you. A `fly deploy --local-only` of the hub that evening would have shipped a
machine that could not boot, from a green-looking local build.

If you build by hand instead, build the base first, every time.

### Decide your app names now

The defaults are `workspacer-hub` and `workspacer-node`. Fly app names are
global, so if either is taken you will find out at `fly apps create` and have to
edit `app =` in the matching `fly.toml`. Check now rather than mid-block:

```sh
fly apps list          # your own apps
```

---

# Part B. The block

Everything below runs in one sitting. The three things only you can do are
marked **GATE**.

## B0. GATE 1: authorize the Fly spend

Fly refuses to create machines or volumes on an org with no payment method, and
it refuses at `fly deploy`, several minutes into an upload. Settle it first.

```sh
fly auth whoami        # should print your email
fly orgs list          # note which org you intend to bill
```

Then open the dashboard and confirm a payment method is on file for that org.
There is no read-only CLI command that reports billing status, so this one is a
browser tab, not a command.

**What you are agreeing to, current as of 2026-08-24 and not re-measured since:**

| | Shape | Cost |
|---|---|---|
| hub | `shared-cpu-1x` / 1 GB, always on | about $6/month (the brief's figure, not one this rehearsal verified) |
| hub volume | 1 GB | about $0.15/month |
| node | `shared-cpu-4x` / 8 GB, asleep by default | $0.0617/hr while awake, so about $9.26 at 150 active hours |
| node volume | 10 GB | $1.50/month, billed **while the machine is stopped** |

Stopped machines are not free: the volume bills continuously and there is a
small rootfs charge on top.

**The node volume is sized at 10 GB, not the container image size.** The node's
container image (about 900 MB base, up to ~2.3 GB with project toolchains
layered on) unpacks to the machine's own ephemeral rootfs and never touches
this volume. What actually lands on `/data` is `$HOME` (dotfiles, SSH keys,
Claude OAuth, a few MB total), the Go/bundle/bun/npm toolchain caches (empty at
first boot, grow with use), and whatever repos and worktrees the operator
clones under `/data/repos`. Nothing in this repo's rehearsal or runbooks
projects a `/data` footprint anywhere near 10 GB. Volumes never shrink, so if a
particular fleet's repos and caches genuinely outgrow it, grow the volume
later rather than guessing high now.

**Set a spend alert before you deploy anything.** There is no way to do this
from `fly.toml` or the CLI: open the Fly dashboard for the org you noted above,
go to its billing settings, and set a spend alert threshold. This is an
account-level setting, so only you can do it, and it is easy to skip because
nothing above forces it. It is a five-minute, one-time step and it is the only
guardrail against a runaway machine or a mistaken deploy costing more than
expected before anyone notices.

## B1. GATE 2: mint two Tailscale auth keys

One visit to the admin console, two keys. Do the ACL first, because a key
carrying a tag that has no owner is rejected at use, not at creation.

**Access controls.** This is the combined policy for both machines. The two
per-machine runbooks each show only their own half; this is both, so you edit
once:

```jsonc
{
  "tagOwners": {
    "tag:workspacer-hub":  ["autogroup:admin"],
    "tag:workspacer-node": ["autogroup:admin"]
  },
  "acls": [
    // the node reaches the hub, and nothing else on your tailnet
    { "action": "accept", "src": ["tag:workspacer-node"],
      "dst": ["tag:workspacer-hub:443,8443"] },
    // your own devices reach the hub. This is how the phone gets /m.
    { "action": "accept", "src": ["autogroup:member"],
      "dst": ["tag:workspacer-hub:443,8443"] },
    // your own devices reach the node, for debugging without `fly ssh console`
    { "action": "accept", "src": ["autogroup:member"],
      "dst": ["tag:workspacer-node:*"] }
  ]
}
```

Note the hub ports. `tailscale serve` terminates TLS and the hub itself binds
`127.0.0.1:7895`, so clients reach **443** and **8443**, never 7895.

**Also check DNS → HTTPS Certificates is enabled.** `tailscale serve` cannot get
a certificate without it, and the hub has no other way in. A hub whose serve
fails is unreachable, and its entrypoint treats that as fatal on purpose.

**Settings → Keys → Generate auth key.** Twice, once per machine:

| Setting | Value | Why |
|---|---|---|
| Reusable | **yes** | the key is presented again if the node key ever expires |
| Ephemeral | **NO** | this is the one that matters. An ephemeral device is removed from the tailnet when it goes offline and **gets a new address when it rejoins**, so the node would churn identity on every wake, and the hub's address is what the node's `HUB_BUS_URL` resolves to |
| Pre-approved | **yes** | only relevant if device approval is on for your tailnet |
| Tags | `tag:workspacer-hub` / `tag:workspacer-node` | load-bearing, not cosmetic: node keys expire after 180 days by default and a sleeping machine cannot reauthenticate itself, but **a device tagged at first authentication has key expiry disabled by default** |
| Expiry | the maximum, 90 days | only used at first boot and at re-auth |

Write both keys into your scratch block. You will not see them again.

## B2. Create both apps, without public IPs

**Do not use `fly launch`.** It allocates public IPs, and neither app should
have one.

```sh
fly apps create workspacer-hub
fly apps create workspacer-node
```

Creating an app allocates nothing and bills nothing. It is safe to do both now
even though the node is not deployed until B6.

## B3. Mint the Fly API token the hub uses to wake the node

```sh
fly tokens create deploy --app workspacer-node --expiry 2160h --name wks-hub-wake
```

Three things about this token, all confirmed against flyctl v0.4.59 during the
rehearsal:

- **The default expiry is 20 years** (`175200h`). Set one. `2160h` is 90 days.
- **Scope it to the NODE's app**, not the hub's and not org-wide, so the blast
  radius is one app's worth of machines.
- **There is no action-scoped token.** No start-only or stop-only token exists.
  Read-only exists at org level and cannot start a machine. Treat this as full
  control of the node app, including creating machines.

## B4. Set the hub's secrets and deploy it

```sh
fly secrets set --app workspacer-hub --stage \
  TAILSCALE_AUTHKEY='tskey-auth-…' \
  FLY_API_TOKEN='FlyV1 fm2_…'
```

`--stage` holds them until the first deploy instead of triggering one.

`HUB_TOKEN` is deliberately **not** set here. Let `bootstrap.sh` mint it on
first boot so you can read it out of the boot log, then decide whether to move
it into `fly secrets set HUB_TOKEN=…` afterwards so it outlives the volume.
Setting it up front works too and is slightly better security hygiene; it just
costs you a second trip if you have not decided.

```sh
fly deploy \
  --config deploy/fly/hub/fly.toml \
  --dockerfile deploy/fly/hub/Dockerfile \
  --app workspacer-hub \
  --local-only \
  .
```

**`--local-only` is required, not optional.** The hub image builds
`FROM workspacer-node-base:dev`, a tag that exists only on your machine. A Fly
remote builder cannot see it. The base has no published home yet and leaving it
local is the current recommendation; see
[`node/BASE_IMAGE.md`](node/BASE_IMAGE.md) § "Identity and tagging" for the
decision that is still open.

This is the first long upload.

### Read the boot log

```sh
fly logs --app workspacer-hub
```

Want, in order:

```
entrypoint: BOOT <id>
bootstrap: FIRST BOOT on this volume — created 12 directories, layout v1.
bootstrap: state guard: checking the create-once files
bootstrap:   remote-token: absent, and nothing on this volume says it ever existed. First run.
  … one line per create-once file …
bootstrap: state guard: no losses detected
bootstrap: FIRST RUN: minted a new pairing credential at /data/home/.config/workspacer/remote-token
bootstrap:   HUB_TOKEN=<32 chars>          ← WRITE THIS DOWN
entrypoint: TAILNET UP after Ns — ipv4=100.x.y.z
entrypoint: MagicDNS name: workspacer-hub.<tailnet>.ts.net
entrypoint: tailscale serve --bg --https=443 → http://127.0.0.1:7895
entrypoint: hub ready (pid …)
entrypoint: probing https://workspacer-hub.<tailnet>.ts.net/health over the whole chain, the way a client uses it
entrypoint: REACHABLE: … answered 200 after Ns.
entrypoint:   DNS, the certificate, serve's proxy and the hub's Host pin are all working.
entrypoint:   A phone on this tailnet can open /m.
entrypoint:   bus     wss://workspacer-hub.<tailnet>.ts.net/bus
entrypoint: BOOT COMPLETE <id>
```

**Write down `HUB_TOKEN` and the MagicDNS name.** The bus line is the node's
`HUB_BUS_URL`, verbatim.

Two lines you will also see and should not worry about:

- `no node registry at …/nodes.json` is expected. You create it in B7.
- `trusted-host=<name>,<name>` repeats the MagicDNS name. Cosmetic. The Host
  pin strips the port before matching, so both the 443 and 8443 origins are
  accepted; the rehearsal checked this directly rather than assuming it.

**Jobs are off on this machine, deliberately.** The entrypoint passes
`--jobs-file ""`. The node's token is operator-tier, operator tier is `trusted`,
and the `jobs.*` gate is a bare `IsTrusted()`, so with jobs on, the node could
schedule a shell job and get `/bin/sh` inside the hub process, which is the
process holding `$FLY_API_TOKEN`. Nothing here schedules a job, so the subsystem
is switched off rather than argued about. Do not turn it on without reading
[`hub/RUNBOOK.md` §9a](hub/RUNBOOK.md).

**The `REACHABLE:` line is the one to actually check.** `tailscale serve`
returning 0 means a config was installed, not that anybody can reach the hub: the
Let's Encrypt certificate for the MagicDNS name is fetched on demand and can fail
after serve has already succeeded, which is this deployment's largest unproven
assumption. So the entrypoint makes one real HTTPS request over the whole chain
and prints the result. That one request exercises DNS, TLS, serve's proxy and the
hub's own Host pin.

If instead you see `WARNING: … did NOT answer`, **the boot has not failed.** The
hub is up and answering on loopback, deliberately: a hub that is up but
unreachable can still be `fly ssh console`'d into, and one that refused to start
cannot. The warning carries its own triage list; `hub/RUNBOOK.md` §7 explains
what each step distinguishes. The short version is that a 403 is the Host pin and
anything else is the certificate or DNS. Nothing else in this runbook would have
told you: the health watchdog polls loopback with a loopback Host header and
never traverses serve at all.

Set snapshot retention now, while you are looking at it. **This volume is the
fleet's single point of failure**: it holds the pairing credential every client
is paired against, the Web Push keypair every phone subscribed to, and the node
registry that is the only wake path.

```sh
fly volumes list --app workspacer-hub
fly volumes update <vol_id> --snapshot-retention 30
```

## B5. Mint the node's bus token, on the hub

```sh
fly ssh console --app workspacer-hub
su - wks
workspacer token create --label fly-node --scope operator
exit
```

The flag is `--label`, not `--name`. (`node/RUNBOOK.md` said `--name` until this
rehearsal ran the command and watched it exit 2.)

`su - wks` matters: the token file location resolves from `XDG_CONFIG_HOME`, and
`wks`'s profile sources the generated `~/.wks-env` so the shell sees exactly
what the daemons see. Run it as root and it lands in the wrong place.

**No hub restart is needed.** The hub re-reads `tokens.json` on every new
connection, so a token created here is live immediately. (`nodes.json` in B7 is
the opposite, and does need one.)

> **Know what you just minted.** An operator-tier scoped token is **trusted** on
> the bus, exactly like the host pairing token. That includes `nodes.wake`. The
> rehearsal proved this directly: an operator-tier scoped token called
> `nodes.wake` and the hub issued a real `POST /v1/apps/…/machines/…/start`.
> So this token can spend money, and anyone who reads it off the node can spend
> yours. It is still the right token to use (it is revocable, which the host
> token is not), but do not treat "scoped" as "limited" here.
> `hub/RUNBOOK.md` §11 currently claims a scoped operator token is refused for
> `nodes.wake`. It is not. See the rehearsal report.

## B6. Set the node's secrets and deploy it

```sh
fly secrets set --app workspacer-node --stage \
  TAILSCALE_AUTHKEY='tskey-auth-…' \
  HUB_BUS_URL='wss://workspacer-hub.<your-tailnet>.ts.net/bus' \
  HUB_TOKEN='<the operator token from B5>'
```

```sh
fly deploy \
  --config deploy/fly/node/fly.toml \
  --dockerfile deploy/fly/node/Dockerfile \
  --app workspacer-node \
  --local-only \
  .
```

This deploys the **base** image, which is workspacer and nothing else: no Go,
Ruby, bun or python3 for application code. That is a working node, not a
development box. To put project toolchains on it, build a small image `FROM`
this one and deploy that instead; [`node/BASE_IMAGE.md`](node/BASE_IMAGE.md) is
the contract and `node/example.Dockerfile` is a buildable skeleton.

Second long upload.

### Read the boot log

```sh
fly logs --app workspacer-node
```

```
=== no previous boot log at /data/logs/prev-boot.log. First boot on this volume, … ===
entrypoint: BOOT <id>
bootstrap: FIRST BOOT on this volume — created 32 directories, layout v1.
bootstrap: state guard: checking the create-once files
bootstrap:   .credentials.json: absent, and nothing on this volume says it ever existed. First run.
  … five more, one per guarded file …
bootstrap: state guard: no losses detected
entrypoint: doorbell listening on :8080 — wake backstop only
entrypoint: starting tailscaled (state=/data/tailscale/tailscaled.state)
entrypoint: authenticating to the tailnet as 'workspacer-node' (first boot, …)
entrypoint: TAILNET UP after Ns — ipv4=100.x.y.z
entrypoint: running claudemon init (hook port 7890)
✓ wrote 12 hook(s) to /data/home/.claude/settings.json
entrypoint: claudemon ready (pid …)
entrypoint: starting brain, attaching to wss://… as a capability provider
entrypoint: BOOT COMPLETE <id>
brain: scope=full, provider for 67 capabilities → hub wss://…
brain: registered 67 method(s)
```

**Write down the tailnet IP.** Part C checks it is the same number.

**Those `absent … First run` lines are the state guard, and they are the guard
working.** None of the files it watches exists yet: the Claude login, the SSH
key and the tailnet identity all arrive in part C. It records that they were
absent; from the boot after they appear, their disappearance is reported, and for
the Claude credential it stops the node rather than letting it come up and hang
every session it is given. `node/RUNBOOK.md` §10 has the file-by-file table and
the way past a refusal.

> **There used to be a false alarm here, and it is gone.** Earlier boots printed
> `brain: STATE LOSS: …/config.yaml is missing` on a genuinely first boot,
> because `bootstrap.sh` pre-creates sibling directories under
> `~/.config/workspacer` and `internal/statelost` counted an empty directory as
> evidence somebody had run there. It now counts only entries that hold
> something: a bare `mkdir` proves a `mkdir` ran, not that a program did.
> `preflight.sh` asserts the absence against a real brain on a real volume. **If
> you see a `brain: STATE LOSS` line now, take it seriously.**

**On the second and every later boot, the log opens with a replay of the previous
one.** The volume's `boot.log` is readable only through a shell on the machine,
and a node that fails to boot does not stay up long enough to give you one, so
the previous boot is replayed to stdout, where `fly logs` can see it, led by a
verdict line saying whether it completed. Lines prefixed `  | ` or `  ! ` are
quoted from that older boot, never this one. Full detail in `node/RUNBOOK.md`
§6, "Where the boot log lives, and why the first line is a replay".

Then confirm kernel-mode Tailscale really came up. Thirty seconds against the
one inference in this design that rests on two facts joined rather than a
quoted sentence:

```sh
fly ssh console --app workspacer-node -C 'ip link show tailscale0'
fly ssh console --app workspacer-node -C 'ls -l /dev/net/tun'
```

And set snapshot retention:

```sh
fly volumes list --app workspacer-node
fly volumes update <vol_id> --snapshot-retention 14
```

## B7. Back to the hub: the node registry

This is the step people think they have already done.

```sh
fly machine list --app workspacer-node    # ← the machine id goes in the file below
fly ssh console --app workspacer-hub
su - wks
cat > ~/.config/workspacer/nodes.json <<'EOF'
[
  {
    "id": "fly-node",
    "label": "Fly node (ord)",
    "fly": {
      "app": "workspacer-node",
      "machineId": "<the id from fly machine list>"
    }
  }
]
EOF
chmod 600 ~/.config/workspacer/nodes.json
exit
fly machine restart <hub_machine_id> --app workspacer-hub
```

**`id` is not free-form.** The node's own `fly.toml` sets `WKS_NODE_ID` to this
same string, and that is how the hub attributes an answering brain to a row
instead of guessing. A hub with ONE node registered guesses correctly anyway, so
a mismatch here is silent until a second node arrives — and then neither node
can complete a wake, because the hub refuses to guess between several on the
path that spends money. Adding a node means editing `WKS_NODE_ID` in that
node's `fly.toml` to match its row here.

**The token is deliberately not in that file.** `nodes.ResolveToken` checks the
entry's inline `token`, then `tokenFile`, then `$FLY_API_TOKEN`, and the
environment is the right answer for a hub deployed on Fly: the credential stays
out of the volume, so a volume snapshot does not copy it, and rotation is
`fly secrets set` rather than an edit inside a running machine.

Confirm in `fly logs`:

```
nodes: 1 node(s) registered from …/nodes.json (1 wakeable)
```

`(0 wakeable)` means the token did not resolve, and the hub can then report the
node but never wake it, which is this machine's only job.

You should **not** see the `--brain-scope` warning. If you do, something is
passing a scope other than `off`, and every node state will be wrong.

> **Worth considering, unproven:** `nodes.json` accepts a `baseUrl`, and the
> value the code itself names is `http://_api.internal:4280`, the same Machines
> API reachable from inside your Fly org's private network. Setting it keeps the
> wake call off the public internet entirely. Nobody has tried it from a real
> Fly machine, so it is a well-founded suggestion rather than a verified one.
> The rehearsal did confirm the field is honoured: pointing `baseUrl` at a dead
> address produced exactly the expected connection error from a real wake call.

## B8. GATE 3: the interactive logins on the node

`fly ssh console` lands you as **root**. Become `wks` first, because its
`.profile` sources the generated `~/.wks-env` and the shell then has exactly the
environment the daemons run with.

```sh
fly ssh console --app workspacer-node
su - wks
env | grep -E '^(HOME|XDG_|GOPATH|BUNDLE_PATH|npm_config_cache)='
```

### Claude Code OAuth, first

```sh
claude
# → /login, follow the URL it prints, paste the code back
```

Then confirm **two** files, not one:

```sh
ls -l ~/.claude/.credentials.json    # the login
ls -l ~/.claude.json                 # onboarding + the per-project trust map
```

`~/.claude.json` is a **sibling** of `~/.claude`, not inside it. It holds
`hasCompletedOnboarding` and `projects[<cwd>].hasTrustDialogAccepted`. Without
it, the first spawn in each project parks on the interactive folder-trust
dialog, which no GUI pane renders and no headless node can answer. The session
just hangs.

### Git

```sh
ssh-keygen -t ed25519 -C 'workspacer-fly-node' -f ~/.ssh/id_ed25519
cat ~/.ssh/id_ed25519.pub          # add as a deploy key or account key
ssh -T git@github.com              # accepts and records the host key
git config --global user.name  'Your Name'
git config --global user.email 'you@example.com'
```

`gh` is installed if you prefer it (`gh auth login`); its token lands in
`~/.config/gh/hosts.yml`, which is on the volume.

### Clone something

```sh
mkdir -p /data/repos && cd /data/repos
git clone git@github.com:you/your-repo.git
```

`/data/repos` and the worktree root (`~/.workspacer/worktrees`) are both on the
volume deliberately. Git worktrees are two-ended: the checkout holds a `.git`
*file* pointing into `<repo>/.git/worktrees/<name>`, and that admin directory
points back. Persist one half and not the other and you get stale admin entries
pointing at dead paths.

**That is the end of the block.** Everything from here can wait.

---

# Part C. The proofs

Nothing above is trustworthy until this passes, and it is the part most likely
to get skipped because the fleet already looks like it works.

Run [`node/RUNBOOK.md` §8](node/RUNBOOK.md) (stop/start, 16 checks) and
[`hub/RUNBOOK.md` §10](hub/RUNBOOK.md) (restart, 12 checks plus the deliberate
state-loss refusal). Do not paraphrase them here; they are the reference.

The four that a casual "does it come up?" test passes while being completely
broken:

| Check | Where | Why it is the one that bites |
|---|---|---|
| Folder trust survived | node §8 check 4 | only a project directory the node has **never seen** exercises `~/.claude.json`'s trust map |
| Cost accounting is non-zero | node §8 check 6 | a refused transcript path fails **silently**, as zeroed usage and an empty transcript |
| The hub's VAPID keypair is unchanged | hub §10 check 4 | a regenerated keypair comes up healthy and every phone still believes it is subscribed |
| Web Push actually arrives | hub §10 check 9 | a subscription negotiated against the old key is refused by the push service, and **nothing client-side reports an error** |

### Add one measurement while you are there

Nobody has answered this and it is cheap once a machine exists:

**When you stop the node, does its bus TCP connection to the hub get severed
cleanly?** Stop the machine and watch the hub's log. If the hub reports the
provider going away (an eviction, a disconnect, `NO PROVIDER for brain.info`),
the connection sent RST or FIN. If the hub says nothing and keeps the provider
registered, a stopped Fly machine goes silent instead, and the hub's
`brain.info` poll is the load-bearing layer rather than a backstop.

The zombie fix does not depend on the answer; it was built two-layered
precisely so it would not. The answer tells you which layer is actually doing
the work. Write it into the brief when you have it.

---

# The wake backstop, and why it is not turned on yet

`fly.toml` sets `auto_start_machines = true` with `auto_stop_machines = "off"`.
Those are two independent keys and the combination is legal and wanted: **the
proxy may wake it, only the hub may sleep it.** It is a token-free backstop for
the day the Fly API token expires.

It does nothing until the app has an IP, and the app has none on purpose.
**The Fly proxy starts the machine before routing the request**, so no
application-level auth on the doorbell can stop a stranger from spending your
money. The wake has already happened by the time your code sees anything. The
only real control is whether the app is routable at all.

When you do turn it on, **Flycast is the shape to use**:

```sh
fly ips list --app workspacer-node          # expect: empty

fly ips allocate-v6 --private --app workspacer-node
```

`--private` allocates a Flycast address reachable only from inside your Fly
org's private network. There is no public doorbell at all. The confirmed flags
on flyctl v0.4.59 are `--private` and `--network <name>`.

The part the node's own runbook does not say, and it is the reason Flycast is
worth doing rather than merely safer: **the hub is on that private network
too.** It is a machine in the same org. So a Flycast doorbell is reachable by
the hub over 6PN without any token and without a public IP, which is exactly
what a backstop is for. A `fly wireguard` peer gets you there from a laptop as
well.

The public alternative (`fly ips allocate-v6` plus
`fly ips allocate-v4 --shared`) makes `workspacer-node.fly.dev` a doorbell
anyone can ring. Do not, until the hub can put the machine back to sleep, and
probably not then.

To turn it off again: release the IPs. Setting `WKS_DOORBELL_ENABLED = "0"`
makes the request 502 **after** the boot, which still woke the machine.

---

# What this rehearsal actually verified

Walked on 2026-08-25 against the current worktree, on a machine with docker, a
logged-in `flyctl` and a live tailnet. Nothing was deployed, created or billed.

**Verified by running it:**

- Both `fly.toml` files parse, and every decision they encode is what this
  document says: region `ord`, node `on-failure`/3, hub `always`, node
  `auto_start_machines = true` with `auto_stop_machines = "off"`, hub with no
  `[http_service]` block and `WKS_HUB_BIND = 127.0.0.1`.
- Both images build clean from the current source. hadolint, shellcheck,
  `docker build --check` and `verify-image.sh` all pass, on the base, the hub
  and the downstream example.
- 63 + 111 bootstrap assertions, on the host and inside the image as `wks` on an
  empty volume.
- **Both entrypoints reach `BOOT COMPLETE` verbatim**, with only `tailscale` and
  `tailscaled` shimmed. That covers the privilege drop, the doorbell, the
  `.wks-env` generation, `claudemon init` (12 hooks and the statusLine
  forwarder), `claudemon serve` and its readiness poll, `brain` with 67
  capabilities, the hub with `--brain-scope off`, and the health watchdog.
- The doorbell answers `GET /health` with 200.
- The hub's Host pins: 200 for the MagicDNS name, 200 for the name with a
  `:8443` suffix, 403 for an unrelated host. `/m` is 200 unguarded, `/app/` is
  401 without a token and 200 with one, `/plugins/origin` advertises the 8443
  origin.
- `nodes.json` loads and reports `1 node(s) registered … (1 wakeable)` with the
  token resolved from `$FLY_API_TOKEN`, no exposure warning at 0600, and no
  `--brain-scope` warning.
- **The node's brain attaches to the hub with an operator-scoped token and
  registers 67 methods.** `nodes.list` then reports the node as `available`,
  with no token anywhere in the payload. Its `lastExit` field now reads as no
  record: the entrypoint consumes the file at boot, so a run that was killed
  without warning cannot be reported as a clean sleep. `node/RUNBOOK.md` §8
  check 12 has the argument and what putting the bus half back would take.
- A second boot takes the populated-volume path and reports
  `previous run ended: {"reason":"signal-TERM",…}` in the boot log, so the
  `last-exit.json` mechanism works.
- `fly` flags used above exist on flyctl v0.4.59: `--stage`, `--local-only`,
  `--snapshot-retention`, `tokens create deploy --app/--expiry/--name`,
  `ips allocate-v6 --private`. `ord` exists; `sea` and `den` do not.

## What a real machine still has to settle

Stated plainly so nobody mistakes silence for verification.

1. **Nothing has been deployed.** No machine, no Fly volume, no server-side
   `fly.toml` validation. The TOML parses and every key was checked against
   flyctl's own struct tags, which is not the same as Fly accepting it.
2. **Kernel-mode `tailscaled` on Fly.** UNPROVEN. The rehearsal shimmed it,
   because a build host has no tun device and no tailnet. `ip link show
   tailscale0` in B6 settles it.
3. **`tailscale up` with a real key.** UNPROVEN, and it is user gate 1. Whether
   a *tagged* device in your tailnet may fetch a Let's Encrypt certificate is
   specifically unconfirmed.
4. **`tailscale serve`.** UNPROVEN, and it is the largest single unknown on the
   hub, which is unreachable if it fails. Also unconfirmed: whether your
   `tailscale` build wants `--bg --https=443 <target>` or the older
   `serve https:443 / <target>` syntax. The entrypoint treats a serve failure as
   fatal and names the three likely causes.
5. **Tailnet IP and MagicDNS stability across stop/start.** The whole design
   assumes it. Part C.
6. **`tailscaled` cold-reconnect time.** Neither vendor publishes a figure. The
   entrypoint measures and logs it (`TAILNET UP after Ns`). This is the single
   most worthwhile day-one number.
7. **Claude Code OAuth, and whether it survives a stop/start.** UNPROVEN, and it
   is user gate 3. If it does not survive, nothing above it matters.
8. **Cost accounting under a relocated `$HOME`.** A refused transcript path
   fails silently as zeroed usage. Part C.
9. **Whether a stopped Fly machine severs its TCP connection cleanly.** The
   measurement described in Part C.
10. **Whether `fly deploy` starts a currently-stopped machine.** Undocumented. A
    deploy also resets the rootfs. Deploy deliberately; test it once on day one.
11. **Whether `bwrap` works in a Firecracker guest.** Needs unprivileged user
    namespaces. Fails closed, so a wrong answer surfaces as a clear refusal the
    first time someone installs a plugin.
12. **Shared-CPU throttling under real builds.** `shared-cpu-4x` gets 20 ms per
    80 ms, about 25% of one core sustained once the 500 s burst balance drains.
    For a box whose job is compiling Go this is the number most likely to make
    it feel broken, and it is easily mistaken for noisy neighbours.
13. **Upload time for the two deploys.** Unmeasured, and it is the dominant term
    in the wall clock for Part B.
14. **arm64, and a Fly remote builder.** Both builds were amd64 and local.
