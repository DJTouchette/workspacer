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
