.PHONY: all build test fmt proto clean dev

RUST_TARGET ?=
GOOS ?= $(shell go env GOOS)
GOARCH ?= $(shell go env GOARCH)

all: build

proto:
	@echo "Generating gRPC code..."
	@mkdir -p crates/rsmgo-pb/src pb
	@which protoc >/dev/null 2>&1 || (echo "protoc not found, install it first"; exit 1)
	@which cargo >/dev/null 2>&1 && cargo install protoc-gen-tonic protoc-gen-prost 2>/dev/null || true
	@protoc \
		--proto_path=proto \
		--prost_out=crates/rsmgo-pb/src \
		--tonic_out=crates/rsmgo-pb/src \
		--tonic_opt=compile_well_known_types \
		proto/rsmgo.proto || true
	@protoc \
		--proto_path=proto \
		--go_out=pb \
		--go-grpc_out=pb \
		--go_opt=paths=source_relative \
		--go-grpc_opt=paths=source_relative \
		proto/rsmgo.proto || true

build-rust:
	cargo build --release

build-go:
	go build -o target/release/rsmgo-control ./control/cmd/rsmgo-control

build-web:
	cd web && pnpm install && pnpm build

desktop:
	cd desktop && pnpm install && pnpm tauri build

build: build-rust build-go build-web

test:
	cargo test
	go test ./...
	cd web && pnpm test

fmt:
	cargo fmt
	gofmt -w .
	cd web && pnpm format

dev:
	@echo "Starting rsmgo development stack..."
	@echo "1. cargo run -p rsmgo-core --bin rsmgo-engine"
	@echo "2. go run ./control/cmd/rsmgo-control"
	@echo "3. cd web && pnpm dev"

clean:
	cargo clean
	rm -rf pb
	rm -rf crates/rsmgo-pb/src/rsmgo
	rm -rf crates/rsmgo-core/src/proto/*
