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

**Amended 2026-08-25, after the first real deploy.** The dry run got most of it
right and seven claims wrong. Each correction is marked where it lives rather
than collected here; the ones worth knowing before you start are that a green
`fly deploy` does not mean the node booted (B6), that B5b does not apply to a
fresh fleet, and that kernel-mode Tailscale on Fly is now settled rather than
assumed.

**Amended again the same evening, after the first full hub+node bring-up.** Four
more, and three of them are flyctl behaving unlike its own vocabulary: a
`--stage`d secret is applied by `fly secrets deploy` and **not** by a restart, no
form of deploy **starts** a stopped machine, and `fly machine restart` refuses on
a crash-looping machine that `fly machine status` calls stopped. All three are in
Part D's preamble, "Three commands that look like a deploy, and are not". The
fourth is B6: its node deploy command ships the **base** image, so running it
against a node built on a project image on top of the base silently takes that
node's toolchains away.

---

## The shape of it

| Part | What | Needs an account? | Your hands-on time |
|---|---|---|---|
| **A. Pre-flight** | build and verify both images | no | one command |
| **B. The block** | the three human gates, and the deploys between them | yes | about 15 minutes |
| **C. The proofs** | stop/start, restart, and the checklists | yes | 20 minutes, later |
| **D. The second time** | redeploy, rotate, restore, tear down | yes | reference, read when needed |

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

**Fill in the expiry column too.** Three of these expire and none of them tell
you in advance. Two of the three fail silently, months after you last thought
about them, so the date you write down here is the only warning you will get.

```
                          value                                      expires
TAILSCALE_AUTHKEY (hub)   tskey-auth-........................        ..........  (mint date + 90d)
TAILSCALE_AUTHKEY (node)  tskey-auth-........................        ..........  (mint date + 90d)
FLY_API_TOKEN             FlyV1 fm2_.........................        ..........  (mint date + 90d)
HUB MagicDNS name         workspacer-hub.<your-tailnet>.ts.net       never
HUB_TOKEN                 ................................ (32 ch)   never
NODE bus token (provider) ................................           never
NODE machine id           ..............                             n/a
```

## The five credentials, and what each one's expiry day looks like

Read this once now. It is the only place the whole set appears together, and
every row is minted in a step below.

| Credential | Where it lives | Expires | The day it expires |
|---|---|---|---|
| **Fly deploy token** (`FLY_API_TOKEN`, B3) | Fly secret on the **hub** | **90 days**, because B3 sets `--expiry 2160h` | Loud and legible. The hub's reconcile loop flips every node to `unreachable` within 30s, with the detail *"the cloud API rejected this hub's credential (expired, revoked, or scoped to a different app)"*. Note that the hub still logs `(1 wakeable)` at boot: **wakeable means a token resolved, never that it works.** Fix: re-run B3 and `fly secrets set FLY_API_TOKEN=…` on the hub. |
| **Tailscale auth key, hub** (B1) | Fly secret on the hub | **90 days**, the maximum B1 tells you to pick | Nothing happens, for months. Normal boots reuse `tailscaled.state` and never touch the key. It only fires on a boot that needs to re-authenticate: a wiped volume, a restored snapshot, a device deleted from the admin console. Then `tailscale up` fails and the entrypoint `die`s, and the hub, whose restart policy is `always`, crash-loops. **The credential you need in a disaster is the one that quietly went dead months ago.** Fix: mint a new key per B1, `fly secrets set TAILSCALE_AUTHKEY=…`. |
| **Tailscale auth key, node** (B1) | Fly secret on the node | **90 days**, same as the hub's | Same latent shape, different ending: the node retries three times and is left `stopped`, which the Machines API cannot distinguish from a node you put to sleep. From a client it reads `waking` → `unreachable`. Fix is the same, minus the crash loop — with one extra step, because the node's key is set with `--stage`: a staged secret is applied by `fly secrets deploy`, **never by a restart**. Part D. |
| **`HUB_TOKEN` / `remote-token`**, the pairing credential (B4) | hub volume, mode `0600`, optionally also a Fly secret | **never** | n/a. It has no expiry field, so nothing forces it to rotate. Two things worth knowing rather than discovering: the boot that mints it also writes it in cleartext into `/data/logs/boot.log`, and every volume snapshot copies that log. Rotation is in Part D. |
| **The node's bus token** (B5, provider tier) | hub `tokens.json`, and a Fly secret on the node | **never**. `authtoken.Mint` takes a scope and a label and has no expiry field | n/a. Provider tier is the reason this is acceptable: the token can register capabilities and answer calls, and it cannot call `nodes.wake` or `nodes.sleep`, so reading it off the node does not let anyone spend your money. Rotation is in Part D, and it is a two-value swap rather than a cutover. |

Your `flyctl` session in `~/.fly/config.yml` also expires, on Fly's own schedule
rather than one you set. That one is harmless: `fly` commands fail with an auth
error and `fly auth login` fixes it in a browser.

The Claude Code OAuth session on the node (B8) is the sixth thing that can go
away, and it is not on this table because it does not expire on a clock. It is
revoked, by a password change or a login elsewhere. See the note at the end of
Part D for what that looks like, which is a hang rather than an error.

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
| **floor**, node never woken | hub + both volumes | **about $7.65/month** |
| **realistic**, node at 150 active hours | the floor plus $9.26 | **about $17/month** |

Stopped machines are not free: the volume bills continuously and there is a
small rootfs charge on top. The floor row is the one to hold on to, because it
is what the fleet costs on a month where you never touch the node, and $1.50 of
it is a number typed once into `fly.toml` that you cannot type down again later.
An always-on node would be $44.44/month on top of the floor, which is why the
node sleeps by default and why B8 and Part C both end by stopping it.

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

Write both keys into your scratch block **with today's date plus 90 days**. You
will not see the keys again, and you will not be told when they die.

An expired auth key is the quietest failure in this deployment. Neither machine
presents its key on a normal boot: `tailscaled.state` is on the volume and gets
reused, so a key that expired months ago costs nothing until a boot needs to
re-authenticate. The boots that need it are exactly the ones you have during a
recovery: a wiped or restored volume, or a device deleted from the admin
console. Then `tailscale up` fails and the entrypoint refuses to continue, on
the hub as a crash loop and on the node as three retries and `stopped`. Minting
a fresh key takes a minute and `fly secrets set TAILSCALE_AUTHKEY=…` applies it;
knowing that is what you need takes much longer if the date is not written down.

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
  **Write the expiry date in your scratch block.** Nothing warns you before it,
  and on the day it lands the hub flips every node to `unreachable` within 30
  seconds with the detail *"the cloud API rejected this hub's credential
  (expired, revoked, or scoped to a different app)"*. That message is good, and
  the boot log is not: the hub still prints `(1 wakeable)`, because wakeable
  means a token resolved, never that it works. Re-minting is this same command
  plus `fly secrets set FLY_API_TOKEN=…` on the hub; see Part D.
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

`--stage` holds them until the first deploy instead of triggering one — until a
**deploy**, and nothing else. Not until a restart, and not until the next boot.
Here that is exactly what you want, because the deploy is the next command. It is
a trap during rotation, and Part D's preamble spells out why.

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
`--jobs-file ""`. The `jobs.*` gate is a bare `IsTrusted()`, and operator tier
is `trusted`, so with jobs on, an operator-tier node could schedule a shell job
and get `/bin/sh` inside the hub process, which is the process holding
`$FLY_API_TOKEN`. Since B5 mints the node at `--scope provider`, which is not
trusted and cannot create `jobs.*` at all, that particular path is already
closed at the tier — but operator-tier tokens still exist, nothing here
schedules a job, and a subsystem nobody uses is cheaper switched off than
argued about. Do not turn it on without reading
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
workspacer token create --label fly-node --scope provider
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

**This token never expires.** `authtoken.Mint` takes a scope and a label and has
no expiry field, so nothing will ever force you to rotate it. Provider tier is
what makes that acceptable rather than merely convenient: the grant below cannot
spend money, so a leaked node token is not a billing incident. Rotate it anyway
if the node's volume is ever exposed, and Part D has the procedure. The same is
true of the hub's own `HUB_TOKEN` from B4.

> **Know what you just minted — and what you did not.** A `provider`-tier token
> may **register capabilities and answer calls**, which is the entire job of a
> node, and it may publish the topics carrying the output of what it registered
> (the fleet feed, statuslines, the PTY stream, visible-terminal requests). That
> is the whole grant.
>
> It **cannot** call `nodes.wake` or `nodes.sleep` (both ask for host authority
> — so this credential cannot spend your money, and cannot stop a machine
> somebody is typing at), cannot spawn agents, cannot write config or files,
> cannot create `jobs.*`, cannot subscribe to *any* event, and does not pass
> `Server.Authorized` — so `POST /plugins/install`, which clones a repo and runs
> its build step **on the hub**, is refused structurally rather than by a point
> guard. Its entire outbound call surface is one method, `layout.get`.
>
> **And the install family is host-only on top of that, as of 2026-08-25.**
> `POST /plugins/install`, `/plugins/examples/install` and `/plugins/reload` run
> code on the HUB's own machine, so they require the hub's own host token and
> answer 403 to *every* scoped token — operator tier included, which is what a
> node deployed before B5b still carries. That is why this step mints a
> **scoped** token and never copies the hub's `remote-token` onto the node: the
> refusal is keyed on the credential, so a node handed the host token would
> simply *be* the host and this gate would see nothing.
>
> **This used to be `--scope operator`, and that is worth remembering rather
> than deleting.** An operator-tier scoped token is `trusted` on the bus,
> exactly like the host pairing token, and the rehearsal proved the consequence
> directly: an operator-tier scoped token called `nodes.wake` and the hub issued
> a real `POST /v1/apps/…/machines/…/start`. The node held nine authorities and
> used one. If you have a node deployed with an operator token, §B5b is the
> swap. (`hub/RUNBOOK.md` §11 agrees with all of this — an earlier edition of
> this runbook flagged a contradiction there that has since been fixed.)

## B5b. Moving a deployed node from an operator token to a provider token

**Skip this step if you are following this runbook in order.** It applies only
to a node that is *already deployed* and holding an operator-tier token from an
earlier edition of these instructions. B5 mints with `--scope provider`, so on a
fresh fleet the node never holds an operator token and there is nothing to swap:
the first real deploy went B5 → B6 and B5b never happened. It is kept here as
the migration for fleets that predate the provider tier.

**No code, image, `fly.toml` or entrypoint change — a secret swap and this
paragraph.** The brain dials with whatever `HUB_TOKEN` it is handed and never
inspects its own tier.

```sh
# on the hub, as wks
workspacer token create --label fly-node --scope provider   # note the new value
# then, from this repo
fly secrets set --app workspacer-node HUB_TOKEN='<the provider token>'
```

Setting a secret **restarts the machine**, so time it deliberately if there is
work in flight on that node. The brain dials back in, sends `register`, and
reads the ack.

**Verify** with this runbook's own criteria: the hub logs the node's
registration, `nodes.list` reports `available`, and `lastExit` reads back over
the bus. A node whose feed is silent while its calls answer means a topic it
publishes has no `Publisher` in `internal/capspec/eventtopics.go` — that is a
one-row fix, not a tier problem.

**Rollback is re-setting the old secret.** The old operator token stays valid
until you revoke it, so this is a two-value swap and not a cutover. Once the
node is confirmed healthy on the new one:

```sh
workspacer token revoke <old-operator-token-or-prefix>
```

`revalidateScoped` closes any socket still holding it within one tick.

## B6. Set the node's secrets and deploy it

```sh
fly secrets set --app workspacer-node --stage \
  TAILSCALE_AUTHKEY='tskey-auth-…' \
  HUB_BUS_URL='wss://workspacer-hub.<your-tailnet>.ts.net/bus' \
  HUB_TOKEN='<the provider token from B5>'
```

`--stage` again holds these for the deploy immediately below. If you ever stage a
secret and then *restart* rather than deploy, the machine boots on the old value;
see Part D.

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

**Which makes the command above the wrong one for a node that already runs a
project image, and wrong silently.** `--dockerfile deploy/fly/node/Dockerfile`
builds and ships the **base** — so running it against a node whose image was
layered on top of the base ships roughly 900 MB of base in place of the image
that was running and discards the layer you added, while flyctl reports a
perfectly ordinary green deploy. **The symptom is a node that comes back up healthy having lost its
toolchains**: `go`, `bun`, `python3`, and whatever else your image installed are
simply gone, and a build that worked yesterday fails with `command not found`.
Nothing warns you, because from Fly's side you asked for exactly this.

So: **B6's command is the bootstrap for a bare base-image node.** If you maintain
your own project image and its own `fly.toml`, redeploy from **your** project
directory with **your** config and **your** Dockerfile. Rebuild the base first
(`preflight.sh` does it), because your `FROM` points at it; then deploy yours.

Second long upload.

**`fly deploy` exiting 0 does not mean the node booted.** The first real deploy
proved this the hard way: flyctl printed `update finished: success` for a
machine whose entrypoint then died. It is not a flyctl bug — `node/fly.toml`
deliberately defines no `[[http_service.checks]]`, because a health check firing
during the tailscaled reconnect would let the platform decide a slow-booting
node is a dead node. With no checks defined, flyctl waits only for the machine
to reach `started`, which means the VM is running, not that anything inside it
worked. **A green deploy and a dead node look identical from the terminal you
deployed from.**

**And it claims even less than that: a green deploy does not prove the machine
was started.** Neither `fly deploy` nor `fly secrets deploy` starts a machine
that is currently stopped — both exit 0 and leave it `stopped` (Part D). What a
green deploy establishes is that the image was uploaded and the machine config
updated. Whether anything is running is a separate question with a separate
command. The boot log below is the verdict; do not go on to B7 until you
have seen `BOOT COMPLETE` in `fly logs`.

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

**That mechanism is no longer theoretical.** On the first real deploy the node
failed to boot, the entrypoint recorded
`{"reason":"boot-failure","exitCode":1}` in `last-exit.json`, and the next boot
replayed the failed boot to stdout where `fly logs` picked it up — which is
exactly the situation it was built for, and the only reason the failure was
diagnosable at all from outside a machine that would not stay up.

It has since earned its keep a second time, on the full bring-up: the prefixes
did the work they were designed for, and a `FATAL` line that read as this boot's
was in fact the previous boot's, replayed. **Trust the `  ! ` and `  | ` margins
over your reading of the log.** A line at the left margin is this boot; a line
behind either prefix is not, however current it looks.

Then confirm kernel-mode Tailscale really came up. **This is settled rather than
open**: on 2026-08-25 machine `1857645df24448` came up with a real `tailscale0`
tun device — kernel mode, not netstack — which is what the design inferred from
two facts joined rather than from a quoted sentence. Run it anyway on your own
machine; it costs thirty seconds and it is the check that tells you which mode
you actually got:

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

## B9. Stop the node

The node has been running since B6, and it bills $0.0617/hr for every hour of
it. Nothing turns it off for you: the hub only sleeps a machine when somebody
asks it to, and `fly.toml` sets `auto_stop_machines = "off"` on purpose.

```sh
fly machine list --app workspacer-node                  # note the machine id
fly machine stop <machine_id> --app workspacer-node
fly machine status <machine_id> --app workspacer-node   # → stopped
```

From here on the product path is the one to use: **Sleep** in the desktop app's
remote-nodes strip, in `/app`, or in `/m`, which calls `nodes.sleep` on the hub.
Both routes end the same way, and both leave the volume, so nothing on it is
lost. Waking is a click; the machine boots in seconds.

Do this whenever you finish a block. A node left running overnight is $1.48 and
a node left running for a week is $10.36, which is more than the rest of the
fleet costs in a month, and there is nothing in the UI that will nag you about
it: a running node reads `available`, which is exactly what a healthy node
should read.

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

### Stop the node again when you are done

`node/RUNBOOK.md` §8 is a stop/start proof, so it **ends with the machine
started** and billing. So does anything you did to answer the measurement above.
Put it back to sleep before you close the laptop, the same way as B9: **Sleep**
in any client, or `fly machine stop <machine_id> --app workspacer-node`.

The hub stays on. It is the always-on half of this design and the only thing
that can wake the node again.

---

# Part D. The second time

Everything above is written for the first deploy. This part is the rest of the
fleet's life: shipping a code change, rotating each credential, getting a volume
back from a snapshot, and taking the whole thing down. None of it is hard, and
all of it is the kind of thing you would rather read now than derive at 2am
against a machine holding your only Claude login.

**The flags below were checked against flyctl v0.4.59 on 2026-08-25** and exist
and spell the way this document spells them: `machine stop -s/--timeout`,
`machine restart --time`, `machine destroy`, `machine update
--mount-point/--restart`, `secrets unset`, `volumes snapshots list`, `volumes
create --snapshot-id/--snapshot-retention`, `volumes destroy`, `apps destroy`,
`ips release`. What has **not** been walked end to end, the way Part B now has,
is the *procedures*: the shape of each is the part that matters and the part
that is hard to work out under pressure.

**Two flags do not spell the way you would guess, and both bite in the middle of
a restart.** `fly machine restart` takes **`--time`**, not `--timeout` — which
is the spelling `fly machine stop` uses for the same idea — and it has **no
`--signal` flag at all**, so a restart cannot be told which signal to send. If
you need a specific signal, do it in two steps:

```sh
fly machine stop <machine_id> --app <app> -s SIGINT --timeout 60
fly machine start <machine_id> --app <app>
```

### Three commands that look like a deploy, and are not

All three were observed on flyctl v0.4.59 during the first full hub+node
bring-up, on 2026-08-25. Each of them prints success. None of them does the thing
its name suggests, and between them they account for most of an evening.

**1. A staged secret is applied by a DEPLOY, and a restart is not a deploy.**
`fly secrets set --stage` puts the value in the app's *pending* set and stops
there. `fly machine restart` then boots the machine with the **old** secret set —
twice in a row, if you let it — while `fly secrets list` reports the new value as
`Staged`, which reads like "present" and means "not applied". The symptom is a
machine re-hitting the same missing-key `FATAL` on every restart with the secret
apparently sitting right there. The command that applies staged secrets without
a full image deploy is:

```sh
fly secrets deploy --app <app>
```

**flyctl tells you this itself, and the footer is authoritative.** `fly secrets
set --stage` prints a line to the effect of *"Deploy with `fly secrets deploy`…"*
under the confirmation. It is not advice about what to do next; it is a statement
about what has and has not been applied. Read it every time.

**2. Neither `fly secrets deploy` nor `fly deploy` starts a stopped machine.**
Both print success against a machine that stays `stopped` — the right behaviour
for a node you deliberately put to sleep, and a baffling one at 1am. Starting it
is a separate command:

```sh
fly machine start <machine_id> --app <app>
```

This is the sharper edge of B6's warning that a green deploy does not prove the
node booted: **it does not even prove the machine was started.** Check
`fly machine status` after every deploy — the same habit D1 asks for below, for
the opposite reason.

**3. `fly machine restart` refuses on a crash-looping machine.** It exits
`failed_precondition: machine still active, refusing to start`, and it says that
**while `fly machine status` reports the machine as `stopped`** — the platform is
still unwinding a restart-policy attempt it has not finished. That is the restart
policy working, not a stuck machine and not something to force. Wait for it to
settle, or drive the two steps yourself, `fly machine stop` then `fly machine
start`, which is the same pair the `--signal` workaround above uses.

## D1. Redeploying after a code change

Four traps, all of them from reading first-deploy instructions as if they were
repeatable.

**Run `preflight.sh` first, every time.** Not because the code changed, but
because `workspacer-node-base:dev` is a mutable local tag and the hub builds
`FROM` it. Preflight always rebuilds the base for exactly this reason. Building
only the image you changed is how you ship a hub on top of a stale base, which
is the near-miss described in Part A.

```sh
./deploy/fly/preflight.sh

fly deploy --config deploy/fly/hub/fly.toml  --dockerfile deploy/fly/hub/Dockerfile  \
  --app workspacer-hub  --local-only .
fly deploy --config deploy/fly/node/fly.toml --dockerfile deploy/fly/node/Dockerfile \
  --app workspacer-node --local-only .
```

**`--local-only` is as required here as it was in B4 and B6**, and for the node
it is still the one that fails quietly rather than loudly.

**The node line above deploys the base image, which may not be your node's
image.** If this node runs a project image layered on the base, redeploying it
with this repo's `node/Dockerfile` swaps your image for the base one and reports
success — see B6 for the symptom. Rebuild the base here, which `preflight.sh`
already does, and then run your own deploy from your own project directory with
your own `fly.toml`. This repo's command updates a bare base-image node and
nothing else.

**Check the machine afterwards — and expect it down, not up.** This is settled
now rather than assumed: **`fly deploy` does not start a stopped machine**, and
neither does `fly secrets deploy`. Both exit 0 and leave it `stopped`.

```sh
fly machine status <machine_id> --app workspacer-node
```

So the check runs in both directions. If the node was asleep and you wanted the
new image *running*, `fly machine start <machine_id>` is a separate command and
nothing will remind you it is missing, because the deploy already said success.
If the node was asleep and you want it to stay that way, this is still the
cheapest habit in Part D — the one that stops a deploy on a Tuesday billing
until Friday.

**Deploying the node ends whatever was running on it.** It replaces the machine.
Time it like a restart, not like a background task.

### D1b. The same deploy, from a release artifact instead of a compile

Everything above compiles brain, workspacer, mcp, claudemon, `hub` and the /app
web bundle on your laptop, every time. CI already built all of that — the
`nightly` prerelease and every `v*` release carry
`workspacer-server-linux-x64.tar.gz`, which holds exactly the files these two
images install. `WKS_INSTALL=artifact` downloads that instead of rebuilding it.

**Which one to use.** Source mode when the code you want on the box is the code
in your worktree — iterating on a daemon, or shipping a commit that was never
released. Artifact mode when you want a *published build* on the box: it is the
same bits every other client got, it needs no Go/Rust/node toolchain at all, and
it turns a ten-minute image build into about twenty seconds. Source mode stays
the default precisely so nothing above this line changes.

```sh
# 1. Decide which release. `nightly` is rolled every night, so pin the commit
#    too — see "SHA and tag" below for why this line is not optional.
TAG=nightly
SHA=$(gh release view "$TAG" --repo DJTouchette/workspacer --json targetCommitish -q .targetCommitish)

# 2. The base, then the hub FROM it. Build both from the SAME tag.
docker build -f deploy/fly/node/Dockerfile \
  --build-arg WKS_INSTALL=artifact \
  --build-arg WKS_RELEASE_TAG="$TAG" --build-arg WKS_RELEASE_SHA="$SHA" \
  -t "workspacer-node-base:$TAG" .

docker build -f deploy/fly/hub/Dockerfile \
  --build-arg WKS_INSTALL=artifact \
  --build-arg WKS_RELEASE_TAG="$TAG" --build-arg WKS_RELEASE_SHA="$SHA" \
  --build-arg WKS_BASE="workspacer-node-base:$TAG" \
  -t "workspacer-hub:$TAG" .

# 3. Deploy the images you just built. --local-only as always, for the same
#    reason as B4/B6 and D1: it fails loudly rather than quietly.
fly deploy --config deploy/fly/hub/fly.toml  --app workspacer-hub \
  --image "workspacer-hub:$TAG"  --local-only
fly deploy --config deploy/fly/node/fly.toml --app workspacer-node \
  --image "workspacer-node-base:$TAG" --local-only

fly machine status <machine_id> --app workspacer-node   # D1's habit, unchanged
```

Build args, all optional except the tag: `WKS_RELEASE_TAG` (default `nightly`),
`WKS_RELEASE_SHA`, `WKS_RELEASE_REPO`, `WKS_RELEASE_ASSET` (pick another
platform's bundle), `WKS_RELEASE_BASE_URL` (point somewhere other than GitHub).
A private repo takes a token as a BuildKit secret, never a build arg, which is
recorded in the image history: `--secret id=gh_token,src=<file>`.

**Preflight still applies, and still builds from source.** `preflight.sh` is
about *this worktree* — it is the check that the code you are holding assembles
and boots. Run it before an artifact deploy too: it also runs the artifact
mode's own 51-assertion suite and, if docker can reach a fixture server, rebuilds
the node image in artifact mode to prove the drift guard fires.
`./deploy/fly/preflight.sh artifact` runs just that part.

**SHA and tag: why the build refuses rather than warns.** A release tag is
mutable, and `nightly` is deliberately deleted and recreated against a new commit
every night. So "I asked for `nightly` and the download succeeded" tells you
nothing about what is now in the image — and neither does the box afterwards:
`workspacer`, `hub` and `brain` have no `--version` flag at all, and `claudemon
--version` prints the Cargo version `0.1.0`, which has not moved in the life of
the project.

So the release bundle carries a `build-stamp` file, and the build **fails**, with
`RELEASE DRIFT` or `COMMIT DRIFT` in the log, when it disagrees with the tag or
sha you asked for. A release published before the stamp existed also fails — it
is exactly the unidentifiable case the stamp is for; build that commit from
source instead.

**Mapping a box back to a commit, and a commit back to a release.** The stamp is
installed at `/usr/local/share/workspacer/build-stamp` (the daemons) and, on the
hub, `/usr/local/share/workspacer/build-stamp.hub` (`hub` + /app). Both are
printed by the entrypoint on **every boot**, so the usual case needs no shell on
the machine at all:

```sh
fly logs --app workspacer-node | grep 'entrypoint:   build:'
#   build: component=server install=release version=0.151.0-nightly.202608270800
#          tag=nightly commit=ccb44ccf… built=2026-08-27T08:00:00Z platform=linux-x64 run=1234

# or, on a machine you can reach:
fly ssh console --app workspacer-node -C 'cat /usr/local/share/workspacer/build-stamp'

# and back the other way — which release shipped that commit:
gh release view nightly --repo DJTouchette/workspacer --json tagName,targetCommitish,publishedAt
gh release list --repo DJTouchette/workspacer --limit 20
```

`install=source` means the image was compiled from a worktree (the `commit=` is
whatever `preflight.sh` passed as `WKS_SOURCE_SHA`, or `unknown` if a bare
`docker build` skipped it). `install=release` means it came off a published
release, and the `tag=` line is the one to hand to `gh release view`.

## D2. Rotating each credential

Four procedures, one per credential that can go bad. All of them assume you have
read "The five credentials" above and know which one you are holding.

**The Fly deploy token** (90 days). Mint a replacement scoped the same way, set
it on the hub, and let the hub restart:

```sh
fly tokens create deploy --app workspacer-node --expiry 2160h --name wks-hub-wake
fly secrets set --app workspacer-hub FLY_API_TOKEN='FlyV1 fm2_…'
```

Setting a secret restarts the app, which is fine on the hub: it is always on and
a restart is seconds. Verify by watching a node go back to `available` in the
strip, not by reading `(N wakeable)` in the boot log, which only says a token
resolved. Revoke the old token in the Fly dashboard once the new one works.

**Either Tailscale auth key** (90 days). Mint per B1 with the same tag and the
same settings, then:

```sh
fly secrets set --app workspacer-hub  TAILSCALE_AUTHKEY='tskey-auth-…'
fly secrets set --app workspacer-node --stage TAILSCALE_AUTHKEY='tskey-auth-…'
```

**Use `--stage` on the node.** Without it, setting the secret restarts the app,
which wakes a sleeping machine and leaves it running and billing.

**Then know what `--stage` defers to.** It holds the value until a **deploy** —
not until the next boot, which is what this paragraph used to claim and what the
first full bring-up disproved. A staged secret plus a `fly machine restart` gives
you a machine booting on the *old* key, as many times as you care to restart it,
while `fly secrets list` shows the new one as `Staged`. So the sequence for a
node you are not otherwise redeploying is stage, apply, and only then start:

```sh
fly secrets set --app workspacer-node --stage TAILSCALE_AUTHKEY='tskey-auth-…'
fly secrets deploy --app workspacer-node               # applies the staged set
fly machine start <machine_id> --app workspacer-node   # only if you want it awake now
```

The middle command is the one that does not start the machine, and the last is
the one that does. A machine already running and already authenticated notices
none of this: the key is only presented at re-auth. That is the case `--stage`
exists for — and it is also why the trap is easy to walk into, because the
rotation you actually need to *apply* is the one on a node that is down.

**The node's bus token** (never expires). It is a two-value swap, not a cutover,
because the old token stays valid until you revoke it:

```sh
# on the hub
fly ssh console --app workspacer-hub
su - wks
workspacer token create --label fly-node --scope provider
exit

# then, from this repo
fly secrets set --app workspacer-node HUB_TOKEN='<the new provider token>'
```

This one has no `--stage`, deliberately: the brain has to reconnect to pick it
up, so you want the restart. Confirm the node re-registers and `nodes.list`
reads `available`, then revoke the old one:

```sh
workspacer token revoke <old-token-or-prefix>
```

`revalidateScoped` closes any socket still holding the revoked token within one
tick.

**The hub's pairing credential, `HUB_TOKEN`** (never expires). This is the
expensive one, because every client is paired against it: the desktop app, the
browser at `/app`, the phone at `/m`, every peer hub with a `peers.json` entry.
Rotating it re-pairs all of them. Do it if the volume was exposed, or if you
shared a volume snapshot, since the boot that minted the token also wrote it in
cleartext into `/data/logs/boot.log` and snapshots copy that log.

```sh
fly secrets set --app workspacer-hub HUB_TOKEN='<a new 32-char value>'
```

Then re-pair each client. **Do not hand-transcribe the old value out of a boot
log to check it.** That is how you end up with a value that differs by one
character from the file on the volume, which the bootstrap detects as an
`IDENTITY CONFLICT` and refuses to boot on, in a loop, because the restart policy
is `always`. Read it out of the machine instead:

```sh
fly ssh console --app workspacer-hub \
  -C 'cat /data/home/.config/workspacer/remote-token'
```

If you do land in that crash loop, `fly secrets unset HUB_TOKEN` fixes it and
needs no shell. The other two escape hatches are `WKS_ALLOW_TOKEN_CHANGE=1`, which
accepts the new value and overwrites the volume's, and `WKS_ALLOW_STATE_LOSS=1`,
which lets a hub boot past a state-guard refusal. Both are deliberate overrides
of a guard that exists for a reason; know which you mean.

## D3. Restoring a volume from a snapshot

Snapshot retention was set deliberately in B4 and B6, and retention you cannot
cash in is a setting rather than a backup. The thing to know before you need it:
**a restore does not restore.** It creates a *new* volume from the snapshot, with
a new id, and the machine has to be pointed at it.

```sh
fly volumes list --app workspacer-hub
fly volumes snapshots list <vol_id>
fly volumes create wks_data --app workspacer-hub --snapshot-id <snap_id> \
  --region ord --size 1 --snapshot-retention 30
```

**`--snapshot-retention` is not optional here, and leaving it off is silent.**
`fly volumes create` defaults it to **5 days**, not to whatever the volume you
are replacing was set to. Restore without it and the hub's 30 days from B4
becomes 5 on the new volume — which is now the only copy of the pairing
credential, the VAPID keypair and the node registry, and the one you are least
likely to check again. The same applies to the node's 14 from B6.

The name must match the `[[mounts]] source` in the app's `fly.toml` (`wks_data`
for both apps) and the region must be `ord`, because the volume pins the machine
to a physical host and `primary_region` is `ord` in both `fly.toml` files. Then
destroy the machine and redeploy so it mounts the new volume, or update the
machine's mount in place: `fly machine update --mount-point` exists on flyctl
v0.4.59, as does `--restart`.

**Two things a restored hub volume will not fix by itself.** The Web Push
keypair is one: if the snapshot predates the current `vapid.json`, or the volume
was empty for a boot, the hub regenerates the keypair and every phone keeps
believing it is subscribed while nothing arrives. There is no client-side error.
Until the client compares its stored key against the server's, the only cure is
to clear the site data or reinstall the PWA on each phone. The second is the
tailnet identity: see D4.

For the **node** volume, the same procedure with `--size 10`,
`--app workspacer-node` and `--snapshot-retention 14` — again explicitly, or the
restore quietly drops B6's 14 days to the 5-day default:

```sh
fly volumes create wks_data --app workspacer-node --snapshot-id <snap_id> \
  --region ord --size 10 --snapshot-retention 14
```

What you get back is the Claude OAuth session, the SSH key, the folder-trust map
in `~/.claude.json` and anything under `/data/repos`.
What a snapshot from before those existed gets you is a node that boots green and
hangs on a login prompt no headless machine can answer, which is why the node's
state guard writes `state/seen` markers: after a restore, read the boot log for
`state guard` lines before trusting the machine.

## D4. Starting over on one machine

Wiping a volume and redeploying is a legitimate move, and the order matters:

1. **Delete the machine's device in the Tailscale admin console first.** This is
   the step that is easy to skip and expensive to skip. `tailscaled` registers as
   a brand-new device on a wiped volume, and while the old device still exists
   the new one gets a suffixed MagicDNS name: `workspacer-hub-1.<tailnet>.ts.net`.
   The entrypoint derives the name rather than trusting a configured one, so the
   hub itself is fine, and the node's `HUB_BUS_URL` secret still names the old
   device, which now resolves to nothing. What you see is a hub that boots
   perfectly and a node that attaches to nothing: `nodes.list` shows it, the wake
   succeeds, the machine starts, and the provider never registers, which surfaces
   as *"the machine was started but its provider did not register"* and points
   your attention at the node when the cause is on the hub.
2. `fly machine destroy <machine_id> --app <app> --force`
3. `fly volumes destroy <vol_id>`
4. Redeploy per B4 or B6. The volume is recreated by the deploy.
5. **Re-read the MagicDNS name from the boot log** and, if it changed, set the
   node's `HUB_BUS_URL` to the new one.

The state guard will not warn you here, and that is correct rather than a gap: a
deliberately-wiped volume has no `state/seen` markers, so it is a clean first
run, and the loss you are looking at is the one you asked for.

## D5. Tearing the whole thing down

Destroying the apps destroys their machines and volumes and stops all billing.
It also destroys the two things that are not recoverable from this repo: the
Claude OAuth session on the node volume, and the pairing credential every client
is paired against.

```sh
fly apps destroy workspacer-node
fly apps destroy workspacer-hub
```

Then clean up what lives outside Fly, because none of it goes away on its own:

- **Delete both devices in the Tailscale admin console.** Otherwise a future
  redeploy gets suffixed MagicDNS names, per D4.
- **Revoke the Fly deploy token** in the Fly dashboard.
- **Revoke the Tailscale auth keys**, if they have not expired.
- **Remove the node from the hub's `nodes.json`**, if the hub is surviving the
  node.
- **Check `fly volumes list` on both apps.** Volumes are the line item that
  outlives a machine, so confirm rather than assume.

## D6. When the node behaves oddly, run this first

Not every failure on this list has a clock behind it. The node's Claude Code
OAuth session is revoked rather than expired, by a password change or a login
somewhere else, and when it goes nothing reports it. The machine boots green,
the brain registers, `nodes.list` says `available`, and every dispatched agent
parks forever on a login prompt no headless machine can answer.

`node/RUNBOOK.md` §8 check 3 is the probe, and it takes one line:

```sh
fly ssh console --app workspacer-node
su - wks
claude -p 'reply with OK'
```

A reply means the credential is alive. A login prompt means redo the Claude
login in `node/RUNBOOK.md` §7a. Run it before you go looking anywhere else,
because it is the failure that looks most like a hang and least like an error.

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
logged-in `flyctl` and a live tailnet. Nothing was deployed, created or billed
**by the rehearsal itself**. The node was deployed for real later the same day,
and what that settled or corrected is marked inline above and summarised at the
end of this section.

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
- **Part D's flags, checked the same way on 2026-08-25**, so Part D is no longer
  the unwalked half of this document as far as spelling goes: `machine stop
  -s/--timeout`, `machine restart --time` (**not** `--timeout`, and there is no
  `--signal`), `machine destroy`, `machine update --mount-point/--restart`,
  `secrets unset`, `volumes snapshots list`, `volumes create
  --snapshot-id/--snapshot-retention` (which **defaults to 5 days**), `volumes
  destroy`, `apps destroy`, `ips release`.
- **What three of those commands actually do, observed during the first full
  hub+node bring-up later on 2026-08-25**: a `--stage`d secret is applied by
  `fly secrets deploy` or a full `fly deploy` and **not** by `fly machine
  restart`, which boots on the old set; **neither** `fly secrets deploy` nor
  `fly deploy` starts a stopped machine, both exit 0 and leave it `stopped`; and
  `fly machine restart` exits `failed_precondition: machine still active,
  refusing to start` against a crash-looping machine that `fly machine status`
  simultaneously calls `stopped`. Part D's preamble carries all three.
- **The hub's TLS certificate is real.** Its first boot fetched a **dns-01**
  Let's Encrypt certificate for the MagicDNS name, and the second boot served the
  cached one instead of re-issuing.
- **Artifact mode (`WKS_INSTALL=artifact`), walked on 2026-08-27** against a
  fixture release served over the docker bridge — not a published one, because
  the release workflow's stamping step ships with this change and no live release
  carries a stamp yet. What ran for real: the node image built in **11 seconds**
  with **neither the `gobuild` nor the `rustbuild` stage executing** (asserted
  from a `--no-cache` build log, not inferred from the Dockerfile); the hub image
  built the same way, skipping the Vite build; both passed the unmodified
  `verify-image.sh`; and **both reached `BOOT COMPLETE`** under the full boot
  rehearsal, refusal, last-exit and jobs sections above, with the same results as
  the source images. The drift guard was made to FIRE three ways: a bundle that
  downloads cleanly but claims a different tag (`RELEASE DRIFT`), the right tag
  built from the wrong commit (`COMMIT DRIFT`), and an artifact hub layered on an
  artifact base from a different commit (`STAMP DRIFT`, from `verify-image.sh`).
  All three fail the build with the reason on stdout.
- **The build stamp reaches the boot log.** Every image now prints one
  `entrypoint:   build: …` line before anything else runs — verified on the
  source node (`install=source commit=ccb44ccf…`), the artifact node, and the
  artifact hub. This is the first honest version answer this deployment has had:
  `workspacer`, `hub` and `brain` still have no `--version` flag, and
  `claudemon --version` still prints `0.1.0`.

**Corrected by the first real deploy, on 2026-08-25:** kernel-mode Tailscale is
settled (open question 2 below), `fly deploy` exits 0 on a node that cannot boot
(B6), `fly.toml`'s `kill_signal`/`kill_timeout` do reach the machine as its stop
defaults, the Machines API spells the restart policy `on-failure` exactly as
`fly.toml` does, B5b never applies to a fresh fleet, and `node/RUNBOOK.md` §8
checks 12 and 16 fired on real hardware.

**Corrected by the first full bring-up, later the same day:** a staged secret is
not applied by a restart, no form of deploy starts a stopped machine, `fly
machine restart` refuses on a crash-looper (all three: Part D and B6), and B6's
node deploy command ships the base image over any project image layered on it
(B6, D1). **Confirmed rather than corrected**, where the text above used to
hedge: the staged-secret footer flyctl prints is authoritative, the previous-boot
replay caught a real misread of which boot a line came from, and the hub's dns-01
certificate provisioned on first boot and was cached for the second.

## What a real machine still has to settle

Stated plainly so nobody mistakes silence for verification.

1. **Partly answered: the node has been deployed for real.** Fly accepted
   `node/fly.toml` server-side and the machine config reads back matching what
   the file says (`{"policy":"on-failure","max_retries":3}`, stop_config
   `{"signal":"SIGINT","timeout":"1m0s"}`). **And the hub has since been deployed too**, later
   the same day: Fly accepted `hub/fly.toml`, the machine booted, and its
   dns-01 Let's Encrypt certificate was issued on the first boot and served from
   cache on the second. What is still unwalked on the hub is `hub/RUNBOOK.md`
   §12, the stop/start proof — the checks that a casual "does it come up?" test
   passes while being completely broken.
2. ~~**Kernel-mode `tailscaled` on Fly.**~~ **SETTLED, and it works.** The
   rehearsal had to shim it, because a build host has no tun device and no
   tailnet. The first real deploy ran `ip link show tailscale0` on machine
   `1857645df24448` and got a real `tailscale0` **tun** device: kernel mode, not
   userspace netstack. The inference the design rested on was correct.
3. ~~**`tailscale up` with a real key.**~~ **ANSWERED on the part that worried
   us.** A *tagged* device in the tailnet may indeed fetch a Let's Encrypt
   certificate: the hub's first boot provisioned one over **dns-01** for its
   MagicDNS name, and its second boot served the cached certificate rather than
   re-issuing. That was user gate 1, and it is through.
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
10. ~~**Whether `fly deploy` starts a currently-stopped machine.**~~
    **ANSWERED: it does not**, and neither does `fly secrets deploy`. Both exit 0
    against a machine that stays `stopped`; `fly machine start` is a separate
    command. A deploy still resets the rootfs, so deploy deliberately — just not
    out of fear that it will wake something.
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
15. **Artifact mode against a REAL GitHub release.** Everything above was proved
    against a fixture served locally. The download path itself (`curl` to
    `https://github.com/…/releases/download/<tag>/…`, redirects, anonymous access
    to a public release, rate limits) has not been exercised, and cannot be until
    a release built by the stamping step exists — **today's `nightly` carries no
    `build-stamp` and no `mcp`, so artifact mode correctly refuses it.** The first
    nightly published after this change lands is what unblocks it.
16. **A box actually deployed from an artifact image.** The images boot under the
    rehearsal; that is assembly plus a local boot, not a Fly machine on a volume
    with a tailnet. `fly deploy --image` with a locally built tag has also not
    been walked — D1b writes it down, D1's warnings about what a green deploy does
    and does not prove apply unchanged.
