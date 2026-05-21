.PHONY: build build-go build-web dev dev-go dev-web install clean check

build: build-go build-web

build-go:
	go build -o workspacer ./cmd/workspacer

build-web:
	cd web && npm run build

dev:
	@trap 'kill 0' EXIT; \
	go run ./cmd/workspacer --web & \
	cd web && npm run dev & \
	wait

dev-go:
	go run ./cmd/workspacer --web

dev-web:
	cd web && npm run dev

install:
	cd web && npm install

clean:
	rm -f workspacer
	rm -rf dist/

check:
	go vet ./...
	cd web && npx tsc --noEmit
