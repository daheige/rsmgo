#!/usr/bin/env bash
set -e

cd /app

case "${1:-}" in
  engine)
    exec ./rsmgo-engine
    ;;
  control)
    exec ./rgo-control
    ;;
  web)
    cd /app/web
    exec node server.js
    ;;
  *)
    echo "Usage: $0 {engine|control|web}"
    echo ""
    echo "This entrypoint is used by docker-compose.yaml to start each service"
    echo "in its own container. To run the legacy all-in-one container, use:"
    echo "  docker run ... --entrypoint /bin/bash rsmgo:latest"
    exit 1
    ;;
esac
