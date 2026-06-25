#!/bin/bash
# ══════════════════════════════════════════════════════════════════════
# ModelSwitch — Cron Setup for Daily Log Archiving
#
# Installs a cron job that runs the archive-logs.sh script daily at 2 AM.
# The script path is resolved relative to this file so it works regardless
# of the current working directory.
#
# Usage: ./scripts/cron-setup.sh
#
# To remove the cron job: crontab -l | grep -v archive-logs.sh | crontab -
# ══════════════════════════════════════════════════════════════════════
set -euo pipefail

# Resolve the directory containing this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARCHIVE_SCRIPT="$SCRIPT_DIR/archive-logs.sh"

# ── Color output ─────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
die()     { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# ── Pre-flight checks ────────────────────────────────────────────────────
if [[ ! -f "$ARCHIVE_SCRIPT" ]]; then
    die "archive-logs.sh not found at: $ARCHIVE_SCRIPT"
fi

# Ensure the script is executable
chmod +x "$ARCHIVE_SCRIPT"

# Check if cron is available
if ! command -v crontab &> /dev/null; then
    die "crontab command not found. Please install cron (e.g., 'sudo apt install cron' on Debian/Ubuntu)"
fi

# ── Install cron job ─────────────────────────────────────────────────────
CRON_ENTRY="0 2 * * * $ARCHIVE_SCRIPT"

# Check if the cron job is already installed
EXISTING_CRON=$(crontab -l 2>/dev/null || true)
if echo "$EXISTING_CRON" | grep -qF "$ARCHIVE_SCRIPT"; then
    info "Cron job already installed, updating..."
    # Remove old entry and add new one
    NEW_CRON=$(echo "$EXISTING_CRON" | grep -v "$ARCHIVE_SCRIPT" || true)
    echo "$NEW_CRON" | { cat; echo "$CRON_ENTRY"; } | crontab -
else
    # Add new entry
    (echo "$EXISTING_CRON"; echo "$CRON_ENTRY") | crontab -
fi

success "Cron job installed: daily log archiving at 2 AM"
info "Script: $ARCHIVE_SCRIPT"
echo ""
echo "To verify: crontab -l"
echo "To remove: crontab -l | grep -v archive-logs.sh | crontab -"
