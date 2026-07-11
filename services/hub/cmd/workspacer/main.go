// Command workspacer is the thin launcher for workspacer's headless server
// mode. `workspacer serve` starts and supervises claudemon (the Rust session
// daemon) and the hub (the Go event bus, run with --brain-scope full so it in
// turn supervises a brain that provides the whole capability surface), wires
// the ports and the shared auth token between them, and prints the URLs +
// pairing token a remote client (/remote, /m, the TUI, MCP) needs. A
// full-scope brain + claudemon IS the headless server — this binary only
// launches, supervises, and reports; it holds no product logic of its own.
//
// `workspacer status` probes what's running; `workspacer install-cli` puts
// this binary on PATH.
package main

import (
	"fmt"
	"os"
)

const usage = `workspacer — headless server launcher for workspacer

Usage:
  workspacer serve        start claudemon + hub (+ brain) and supervise them
  workspacer status       report what's running on the workspacer ports
  workspacer install-cli  put this binary on your PATH
  workspacer help         show this help

Run "workspacer <command> -h" for the command's flags.
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}
	switch os.Args[1] {
	case "serve":
		os.Exit(runServe(os.Args[2:]))
	case "status":
		os.Exit(runStatus(os.Args[2:]))
	case "install-cli":
		os.Exit(runInstallCLI(os.Args[2:]))
	case "help", "-h", "--help":
		fmt.Print(usage)
	default:
		fmt.Fprintf(os.Stderr, "workspacer: unknown command %q\n\n%s", os.Args[1], usage)
		os.Exit(2)
	}
}
