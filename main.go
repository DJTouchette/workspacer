package main

import (
	"embed"
	_ "embed"
	"log"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// Wails uses Go's `embed` package to embed the frontend files into the binary.
// Any files in the frontend/dist folder will be embedded into the binary and
// made available to the frontend.
// See https://pkg.go.dev/embed for more information.

//go:embed all:frontend/dist
var assets embed.FS

// main function serves as the application's entry point. It initializes the application,
// creates a window, and starts the Workspacer app.
func main() {

	// Create a new Wails application by providing the necessary options.
	app := application.New(application.Options{
		Name:        "Workspacer",
		Description: "A horizontal-scroll workspace for agent-driven development",
		Services: []application.Service{
			application.NewService(&WorkspaceService{}),
			application.NewService(&TerminalService{}),
			application.NewService(&ConfigService{}),
		},
		Assets: application.AssetOptions{
			Handler: application.AssetFileServerFS(assets),
		},
		Mac: application.MacOptions{
			ApplicationShouldTerminateAfterLastWindowClosed: true,
		},
		Linux: application.LinuxOptions{
			ProgramName: "workspacer",
		},
	})

	// Create the main window with appropriate sizing for a workspace IDE.
	app.Window.NewWithOptions(application.WebviewWindowOptions{
		Title:     "Workspacer",
		Width:     1600,
		Height:    900,
		MinWidth:  800,
		MinHeight: 600,
		Frameless: false,
		Mac: application.MacWindow{
			InvisibleTitleBarHeight: 50,
			Backdrop:                application.MacBackdropTranslucent,
			TitleBar:                application.MacTitleBarHiddenInset,
		},
		Linux: application.LinuxWindow{
			WebviewGpuPolicy: application.WebviewGpuPolicyOnDemand,
		},
		BackgroundColour: application.NewRGB(27, 38, 54),
		URL:              "/",
	})

	// Run the application. This blocks until the application has been exited.
	err := app.Run()

	// If an error occurred while running the application, log it and exit.
	if err != nil {
		log.Fatal(err)
	}
}
