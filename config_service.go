package main

import (
	"context"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// ConfigService exposes configuration to the frontend.
type ConfigService struct {
	config Config
}

func (cs *ConfigService) ServiceStartup(ctx context.Context, options application.ServiceOptions) error {
	cs.config = LoadConfig()
	return nil
}

// GetConfig returns the full configuration.
func (cs *ConfigService) GetConfig() Config {
	return cs.config
}

// GetConfigPath returns the path to the config file so the user can edit it.
func (cs *ConfigService) GetConfigPath() string {
	return ConfigPath()
}

// ReloadConfig re-reads the config file from disk.
func (cs *ConfigService) ReloadConfig() Config {
	cs.config = LoadConfig()
	return cs.config
}
