package main

import _ "embed"

// remoteHTML is the self-contained mobile remote-control client served at
// /remote. It speaks the bus protocol over /bus — no build step, no bundler.
//
//go:embed remote.html
var remoteHTML []byte
