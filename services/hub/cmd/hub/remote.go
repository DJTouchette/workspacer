package main

import _ "embed"

// remoteHTML is the self-contained mobile remote-control client served at
// /remote. It speaks the bus protocol over /bus — no build step, no bundler.
//
//go:embed remote.html
var remoteHTML []byte

// mobileHTML is the mobile-first remote client served at /m — an
// attention/decision-first companion to /remote (fleet glance, a "Needs You"
// approval/question queue, and per-agent chat). Self-contained, speaks the bus
// protocol over /bus, no build step.
//
//go:embed mobile.html
var mobileHTML []byte

// xterm.js + fit addon + css, vendored from @xterm/xterm so the remote can
// render a live terminal mirror with no CDN dependency (works offline / over
// Tailscale). Served unguarded as static library assets.
//
//go:embed xterm.js
var xtermJS []byte

//go:embed xterm.css
var xtermCSS []byte

//go:embed addon-fit.js
var addonFitJS []byte

// PWA assets for the /m mobile client: web manifest, service worker (background
// Web Push + shell cache), and app icons. Served unguarded — the browser
// fetches manifest/SW/icons without our bus token, and none carry secrets (the
// real boundary is /bus).
//
//go:embed manifest.webmanifest
var manifestJSON []byte

//go:embed sw.js
var swJS []byte

//go:embed icon-192.png
var icon192 []byte

//go:embed icon-512.png
var icon512 []byte

//go:embed icon-maskable-512.png
var iconMaskable512 []byte

//go:embed apple-touch-icon.png
var appleTouchIcon []byte
