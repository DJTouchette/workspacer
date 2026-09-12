package main

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/djtouchette/workspacer-hub/internal/uploads"
	"time"
)

const maxUploadBytes = uploads.MaxBytes

var uploadDirName = uploads.DirName()

type filesUploadParams = uploads.Params

func rpcFilesUpload(params json.RawMessage) (any, error) { return uploads.Store(params) }

// An isolated worker must own its private attachment file. Never fall back to
// hub-owned bytes when that receiver is unavailable or its ACK is uncertain.
func routedFilesUpload(call func(context.Context, string, any) (json.RawMessage, error), worker bool) func(json.RawMessage) (any, error) {
	return func(params json.RawMessage) (any, error) {
		if !worker {
			return rpcFilesUpload(params)
		}
		if len(params) > (maxUploadBytes/3+1)*4+16*1024 {
			return nil, fmt.Errorf("files.upload: payload too large")
		}
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		return call(ctx, "files.receiveUpload", params)
	}
}
