# Running workspacer in Docker

The container runs workspacer **headless** — `workspacer serve` supervising
claudemon (sessions), the hub (event bus + web clients) and a full-scope brain.
It is the same stack the `workspacer-server-*` release tarball ships, and it
serves the *complete* React app at `/app`, not a cut-down web UI.

There is no Electron in the image. The desktop app stays a desktop app; this is
for hosting the fleet somewhere else and driving it from a browser, a phone, or
`wks-tui`.

## Pull and run

```sh
docker run -d --name workspacer \
  -p 127.0.0.1:7895:7895 \
  -v "$PWD":/workspace \
  -v workspacer-home:/home/wks \
  ghcr.io/djtouchette/workspacer:latest

docker logs workspacer     # the banner prints your URLs + pairing token
```

The banner prints the token; open the app with it in the query string:

```
http://localhost:7895/app/?token=<token from the banner>
```

The token is required — a bare `/app/` answers 401, it does not prompt. (Only
the entry document is guarded; the hashed asset bundle is public and cached, as
the real boundary is the `/bus` websocket.) The banner also prints ready-made
`/remote` and `/m` URLs with the token already attached.

Tags: `latest` (newest release), `X.Y.Z` / `X.Y` / `X` (pinned), `edge` (master).
Images are multi-arch — `linux/amd64` and `linux/arm64`.

Or with compose, from a checkout:

```sh
docker compose up -d
docker compose logs -f
```

## What you get on port 7895

| Path      | Client                                              |
|-----------|-----------------------------------------------------|
| `/app`    | the full renderer — real parity with the desktop app |
| `/m`      | the mobile PWA (installable, web push)               |
| `/remote` | the lightweight terminal client                      |
| `/`       | the hub bus itself (`wks-tui --bus`, MCP, plugins)   |

claudemon's own ports (7891 API, 7890 hooks) deliberately stay on loopback
*inside* the container — they are unauthenticated, and only the hub is meant to
face a network.

## Authenticating the agent CLI

Claude Code is installed in the image, but it has no credentials. Either:

**Reuse your host login** — mount it read-write (the CLI refreshes tokens):

```sh
-v "$HOME/.claude":/home/wks/.claude
```

**Or log in inside the container**, once, into the named volume:

```sh
docker exec -it workspacer claude
```

**Or use an API key**: `-e ANTHROPIC_API_KEY=sk-ant-…`.

Other providers (codex, opencode, pi) aren't installed by default — add them to
the image with `--build-arg AGENT_CLIS="@anthropic-ai/claude-code @openai/codex"`,
or `docker exec … npm i -g` into a running container.

## Exposing it beyond localhost

The examples publish to `127.0.0.1` on purpose. Inside the container the server
binds `0.0.0.0` (it has to, or the published port would reach nothing), and the
hub requires the bearer token for every bus operation — but **that token is the
only thing between the internet and an agent with a shell on your machine.**

Put it on a private network. Tailscale is the intended path:

```sh
# publish to the tailnet interface only
-p 100.x.y.z:7895:7895
```

`/m` needs HTTPS for web push — `tailscale serve` in front of the container
gives you that without a public certificate.

Pin the token instead of letting it be minted, if you're automating:

```sh
-e HUB_TOKEN="$(openssl rand -base64 24 | tr '+/' '-_' | tr -d '=')"
```

Do not put this on a public IP with a port forward.

## Volumes

Two mounts: your code, and the server's home directory.

```sh
-v "$PWD":/workspace       # what the agents work on
-v workspacer-home:/home/wks   # everything the server needs to survive a restart
```

`/home/wks` is one volume rather than several because durable state is spread
across sibling directories:

| Path                            | Holds                                          |
|---------------------------------|------------------------------------------------|
| `.config/workspacer`            | pairing token, `config.yaml`, installed plugins |
| `.config/workspacer-hub`        | pane layout, the VAPID keypair `/m` push needs  |
| `.claudemon`                    | the session SQLite store                        |
| `.claude`                       | agent CLI credentials, transcripts, settings    |
| `.workspacer`                   | handoff briefs, model-rate overrides            |

Mount the whole home and none of it can be missed. Lose
`.config/workspacer-hub` and every phone re-subscribes to push; lose
`.claudemon` and your resumable sessions are gone.

`.config/workspacer` is the same XDG location the desktop app uses, so a phone
paired against your desktop keeps working against the container, and vice versa.

## File ownership

The container runs as uid 1000 (`wks`). If your host uid differs, a bind-mounted
`/workspace` will be read-only to the agents. Run as yourself:

```sh
--user "$(id -u):$(id -g)"
```

The entrypoint warns on startup when a mounted directory isn't writable, rather
than letting it surface later as an opaque daemon error.

## Other commands

Everything after the image name is a `workspacer` subcommand:

```sh
docker exec workspacer workspacer status
docker run --rm -v workspacer-home:/home/wks \
  ghcr.io/djtouchette/workspacer:latest token list
```

## Building it yourself

```sh
make docker-build          # or: docker build -t workspacer:local .
```

The build context is the repo root. Three stages build in parallel — the web
renderer (Node), the Go daemons, and claudemon (Rust) — into a slim Node
runtime. A cold build compiles a release Rust binary, so expect several minutes;
BuildKit cache mounts make rebuilds much faster.

## Limits

- **No desktop app, no Electron.** Browser/phone/TUI clients only.
- **Agents run as root-equivalent inside the container.** The container is the
  sandbox boundary; there is no second one inside it.
- **Plugin OS sandboxing** (`WORKSPACER_PLUGIN_SANDBOX`) relies on host
  facilities that may not be available in every container runtime.
- **One server per config volume.** Two containers sharing a config volume will
  fight over the token file and the session store.
