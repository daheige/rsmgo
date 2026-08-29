# syntax=docker/dockerfile:1

# Multi-stage Dockerfile for rsmgo.
# Builds the Rust engine, Go control plane, and Next.js web UI.
# Can run all services in one container (legacy) or be used by docker-compose.yaml
# to start engine / control / web as separate services.

# -----------------------------------------------------------------------------
# Stage 1: Build the Rust engine (rsmgo-engine)
# -----------------------------------------------------------------------------
FROM rust:1.98.0-bullseye AS rust-builder

WORKDIR /build

# Install native dependencies required by reqwest / openssl.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config \
       libssl-dev \
       ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY crates ./crates

RUN cargo build --release -p rsmgo-core --bin rsmgo-engine

# -----------------------------------------------------------------------------
# Stage 2: Build the Go control plane (rgo-control)
# -----------------------------------------------------------------------------
FROM golang:1.26.4-bullseye AS go-builder

WORKDIR /build

COPY go.mod go.sum ./
COPY control ./control
COPY pb ./pb

RUN CGO_ENABLED=0 go build -o rgo-control ./control/cmd/rsmgo-control

# -----------------------------------------------------------------------------
# Stage 3: Build the Next.js web UI
# -----------------------------------------------------------------------------
FROM node:22-slim AS web-builder

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
RUN corepack enable && corepack prepare pnpm@9.0.0 --activate

WORKDIR /build/web

COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile

COPY web ./

ARG RSMGO_CONTROL_URL=http://localhost:9090
ENV NEXT_TELEMETRY_DISABLED=1
# The rewrites in next.config.js need to know where the control plane is.
ENV RSMGO_CONTROL_URL=${RSMGO_CONTROL_URL}

RUN pnpm build

# -----------------------------------------------------------------------------
# Stage 4: Runtime image
# -----------------------------------------------------------------------------
FROM node:22-bullseye-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy compiled binaries.
COPY --from=rust-builder /build/target/release/rsmgo-engine /app/
COPY --from=go-builder /build/rgo-control /app/

# Copy the standalone Next.js server and static assets.
COPY --from=web-builder /build/web/.next/standalone /app/web/
COPY --from=web-builder /build/web/public /app/web/public
COPY --from=web-builder /build/web/.next/static /app/web/.next/static

# Copy default configuration and make engine listen on all interfaces inside the container.
COPY app.yaml /app/app.yaml
RUN sed -i \
    's/127\.0\.0\.1:50051/0.0.0.0:50051/g; s/127\.0\.0\.1:8080/0.0.0.0:8080/g' \
    /app/app.yaml

ENV RSMGO_CONFIG=/app/app.yaml
# Make the Next.js standalone server listen on the same port as `pnpm dev`.
ENV PORT=1338

RUN mkdir -p /app/share/rsmgo
VOLUME ["/app/share/rsmgo"]

EXPOSE 50051 8080 9090 1338

COPY docker-entrypoint.sh /app/
RUN chmod +x /app/docker-entrypoint.sh

ENTRYPOINT ["/app/docker-entrypoint.sh"]
