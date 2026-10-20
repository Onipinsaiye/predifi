<#
.SYNOPSIS
  PrediFi - One-command developer environment helper for Windows PowerShell.

.DESCRIPTION
  Provides streamlined commands to start, stop, seed, monitor, and clean the
  PrediFi local full-stack environment.

.EXAMPLE
  .\scripts\dev.ps1 up
  .\scripts\dev.ps1 down
  .\scripts\dev.ps1 clean
  .\scripts\dev.ps1 seed -- --fresh
  .\scripts\dev.ps1 status
#>

[CmdletBinding()]
param (
    [Parameter(Position = 0)]
    [ValidateSet("up", "down", "clean", "seed", "status", "logs", "logging", "help")]
    [string]$Command = "up",

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ArgsList
)

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

switch ($Command) {
    "up" {
        Write-Host "Starting PrediFi full-stack environment..." -ForegroundColor Cyan
        docker compose up @ArgsList
    }
    "down" {
        Write-Host "Stopping PrediFi environment..." -ForegroundColor Yellow
        docker compose down @ArgsList
    }
    "clean" {
        Write-Host "Cleaning up containers and volumes..." -ForegroundColor Red
        docker compose down -v --remove-orphans
        Write-Host "Clean complete. Next 'up' will re-seed from scratch." -ForegroundColor Green
    }
    "seed" {
        Write-Host "Running database seeder..." -ForegroundColor Cyan
        docker compose run --rm db-seed cargo run --bin predifi-seed -- @ArgsList
    }
    "status" {
        docker compose ps
    }
    "logs" {
        docker compose logs -f @ArgsList
    }
    "logging" {
        Write-Host "Starting PrediFi with logging stack (Loki, Promtail, Grafana)..." -ForegroundColor Cyan
        docker compose --profile logging up @ArgsList
    }
    "help" {
        Write-Host "PrediFi Developer Environment CLI" -ForegroundColor Green
        Write-Host ""
        Write-Host "Usage: .\scripts\dev.ps1 [command] [args]"
        Write-Host ""
        Write-Host "Commands:"
        Write-Host "  up           Start full stack (default)"
        Write-Host "  down         Stop all running containers"
        Write-Host "  clean        Stop containers and remove volumes (resets DB & seed data)"
        Write-Host "  seed [args]  Run/re-run database seeder (e.g. .\scripts\dev.ps1 seed -- --fresh)"
        Write-Host "  status       Display status and health check results"
        Write-Host "  logs [svc]   View and stream logs"
        Write-Host "  logging      Start stack with Loki/Promtail/Grafana profile"
        Write-Host "  help         Print this help message"
    }
}
