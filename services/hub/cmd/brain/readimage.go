package main

// `fs.readImage` — composer image thumbnails, headless.
//
// TWIN: apps/desktop/src/main/services/imagePreview.ts (readImagePreview) and
// the fs.readImage registration in hubCapabilities.ts.
//
// The desktop registers this with registerCapability rather than `cat`
// PRECISELY because the brain had no counterpart — its own comment says so, and
// the delegation guard's header records that shipping it behind `cat` once
// killed every remote thumbnail. This is that counterpart.
//
// WHAT THIS PROVIDER CAN AND CANNOT DO, stated rather than implied.
//
// The desktop decodes with Electron's nativeImage and hands back a DOWNSCALED
// thumbnail. There is no nativeImage here, and no decoder in the standard
// library that resizes, so this provider serves the twin's OTHER branch — the
// verbatim-bytes fallback it already has for SVG and for formats nativeImage
// declines — under the twin's own cap of MAX_INLINE_BYTES. The consequences,
// both real:
//
//   - An image under the inline cap renders IDENTICALLY: the renderer gets a
//     data: URL it can draw, which is all the composer tile needs.
//   - An image OVER it is refused here where the desktop would have thumbnailed
//     it. The caller's fallback for a rejection is a plain file chip — the same
//     thing it shows for an unreadable image today — so the failure is a
//     smaller preview, not a broken one. Raising the cap is not the fix;
//     shipping a 12 MP photo through a WebSocket to draw a 56px tile is what
//     the thumbnail exists to avoid.
//
// THE ALLOWLIST IS THE SECOND HALF OF THE CONFINEMENT and is why this file
// cannot be "read the file, base64 it". assertPathAllowed holds the path to the
// workspace roots, but within a root the extension allowlist is what stops this
// capability being a general-purpose file reader with a data: URL wrapper — a
// caller could otherwise base64 any file in an agent cwd through a method whose
// name says "image". The desktop's MIME_BY_EXT doubles as that allowlist and
// says so; the same list is here, minus the entries no browser renders, because
// this provider only ever takes the inline path.
//
// The pixel cap is kept for the formats whose headers the standard library
// parses. It bounds the CLIENT's decode rather than ours (nothing here decodes
// pixels), which is a weaker reason than the desktop's — but a 20000×20000 PNG
// of flat colour is ~91 KB on disk and 1.6 GB decoded, and the browser being
// the one that falls over does not make it a good outcome.

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"image"
	_ "image/gif"  // registers the GIF header decoder for DecodeConfig
	_ "image/jpeg" // …and JPEG
	_ "image/png"  // …and PNG
	"io"
	"os"
	"path/filepath"
	"strings"
)

// maxInlineImageBytes is the twin's MAX_INLINE_BYTES: these bytes reach the
// renderer as-is, so the cap is on what is worth sending, not on what is safe
// to open.
const maxInlineImageBytes = 2 * 1024 * 1024

// maxImagePixels is the twin's MAX_SOURCE_PIXELS — 40 MP clears a
// 50-megapixel-era camera photo while refusing the pathological cases.
const maxImagePixels = 40_000_000

// inlineImageMime is the extension allowlist, and it is deliberately the
// INTERSECTION of the twin's MIME_BY_EXT with its BROWSER_RENDERABLE set.
//
// TIFF is the entry that is absent and the reason the intersection is the right
// list: no browser displays it, so inlining it would produce a broken-image
// tile where a rejection gives the honest plain chip. The desktop can serve
// TIFF only because it decodes and re-encodes it.
var inlineImageMime = map[string]string{
	"png":  "image/png",
	"jpg":  "image/jpeg",
	"jpeg": "image/jpeg",
	"gif":  "image/gif",
	"webp": "image/webp",
	"svg":  "image/svg+xml",
	"bmp":  "image/bmp",
	"ico":  "image/x-icon",
	"avif": "image/avif",
}

// imagePreview is the wire shape. TWIN: ImagePreview.
//
// Width/height are 0 when the format's header was not parsed — which the twin
// also reports for its inline branch, and which the renderer already handles
// (SVG has no intrinsic pixel size at all).
type imagePreview struct {
	Path    string `json:"path"`
	DataURL string `json:"dataUrl"`
	Width   int    `json:"width"`
	Height  int    `json:"height"`
	Size    int64  `json:"size"`
}

// readImagePreview reads an allowlisted image at an ALREADY-CONFINED path and
// returns it as a data: URL. It does no authorization — the guard lives at the
// capability, the way it does on the desktop.
func readImagePreview(p string) (*imagePreview, error) {
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(p), "."))
	mime, ok := inlineImageMime[ext]
	if !ok {
		return nil, fmt.Errorf("not a previewable image: %s", p)
	}
	st, err := os.Stat(p)
	if err != nil {
		return nil, err
	}
	if !st.Mode().IsRegular() {
		// A directory, a device node or a fifo. Refused before any read: opening
		// a fifo inside an agent cwd would block this handler forever.
		return nil, fmt.Errorf("not a regular file: %s", p)
	}
	if st.Size() > maxInlineImageBytes {
		return nil, fmt.Errorf("image is %d bytes and this provider inlines the original (max %d) — "+
			"the desktop provider downscales instead; there is no image decoder here to resize with", st.Size(), maxInlineImageBytes)
	}

	f, err := os.Open(p)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	// LimitReader as well as the stat: the file can grow between the two, and
	// the cap has to hold on the bytes actually sent rather than on the bytes
	// that were there a moment ago.
	data, err := io.ReadAll(io.LimitReader(f, maxInlineImageBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(data)) > maxInlineImageBytes {
		return nil, fmt.Errorf("image grew past %d bytes while being read", maxInlineImageBytes)
	}

	// Header-only decode: DecodeConfig reads dimensions without allocating a
	// pixel buffer, so the bomb check itself cannot be the bomb. A format the
	// standard library does not register (svg, webp, bmp, ico, avif) reports no
	// dimensions and passes, bounded by the byte cap alone — which is the
	// twin's behaviour for the formats its own header prober does not know.
	width, height := 0, 0
	if cfg, _, err := image.DecodeConfig(bytes.NewReader(data)); err == nil {
		width, height = cfg.Width, cfg.Height
		if width > 0 && height > 0 && width*height > maxImagePixels {
			return nil, fmt.Errorf("image is %d×%d (max %d pixels)", width, height, maxImagePixels)
		}
	}

	return &imagePreview{
		Path:    p,
		DataURL: "data:" + mime + ";base64," + base64.StdEncoding.EncodeToString(data),
		Width:   width,
		Height:  height,
		Size:    st.Size(),
	}, nil
}

func (r *registry) readImage(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path string `json:"path"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Path == "" {
		return nil, fmt.Errorf("fs.readImage requires a path")
	}
	// The CANONICAL path the guard returned is the one opened — never the
	// caller's string.
	canonical, err := assertPathAllowed("fs.readImage", p.Path, r.workspaceRoots(ctx))
	if err != nil {
		return nil, err
	}
	preview, err := readImagePreview(canonical)
	if err != nil {
		return nil, err
	}
	return jsonResult(preview)
}
