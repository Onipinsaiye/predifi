#!/usr/bin/env bash
# scripts/dev.sh
#
# PrediFi — One-command developer environment helper.
#
# Usage:
#   ./scripts/dev.sh [command] [options]
#
# Commands:
#   up         Start the full local stack (postgres, redis, stellar, db-seed, backend, frontend)
#   down       Stop all services
#   clean      Stop services and wipe all volumes (fresh start)
#   seed       Run database seeder (options: --fresh, --scenario <name>, --num-pools <n>)
#   status     Show running containers and health status
#   logs       Tail container logs (e.g. ./scripts/dev.sh logs backend)
#   logging    Start full stack with Loki + Promtail + Grafana
#   help       Show this help message

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

CMD="${1:-up}"
shift || true

case "${CMD}" in
  up)
    echo "Starting PrediFi full-stack environment..."
    docker compose up "$@"
    ;;
  down)
    echo "Stopping PrediFi environment..."
    docker compose down "$@"
    ;;
  clean)
    echo "Cleaning up containers and volumes..."
    docker compose down -v --remove-orphans
    echo "Clean complete. Next 'up' will re-seed from scratch."
    ;;
  seed)
    echo "Running database seeder..."
    docker compose run --rm db-seed cargo run --bin predifi-seed -- "$@"
    ;;
  status)
    docker compose ps
    ;;
  logs)
    docker compose logs -f "$@"
    ;;
  logging)
    echo "Starting PrediFi environment with logging stack (Loki, Promtail, Grafana)..."
    docker compose --profile logging up "$@"
    ;;
  -h|--help|help)
    echo "PrediFi Developer Environment CLI"
    echo ""
    echo "Usage: ./scripts/dev.sh [command] [args]"
    echo ""
    echo "Commands:"
    echo "  up           Start full stack (default)"
    echo "  down         Stop all running containers"
    echo "  clean        Stop containers and remove volumes (resets DB & seed data)"
    echo "  seed [args]  Run/re-run database seeder (e.g. ./scripts/dev.sh seed -- --fresh)"
    echo "  status       Display status and health check results"
    echo "  logs [svc]   View and stream logs"
    echo "  logging      Start stack with Loki/Promtail/Grafana logging profile"
    echo "  help         Print this help message"
    ;;
  *)
    echo "Unknown command: ${CMD}"
    echo "Run './scripts/dev.sh help' for usage."
    exit 1
    ;;
esac
