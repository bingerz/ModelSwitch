#!/bin/bash
# ══════════════════════════════════════════════════════════════════════
# ModelSwitch — Log Archiving Script
#
# Compresses and archives NDJSON log files older than the retention period
# to a dedicated archive directory. Archives older than the cleanup period
# are deleted automatically.
#
# Usage: ./scripts/archive-logs.sh
#
# Environment variables:
#   MODELSWITCH_CONFIG_DIR  Data directory (default: ~/.config/modelswitch)
#   LOG_RETENTION_DAYS      Days before archiving (default: 30)
#   ARCHIVE_RETENTION_DAYS  Days before deleting archives (default: 180)
# ══════════════════════════════════════════════════════════════════════
set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────
DATA_DIR="${MODELSWITCH_CONFIG_DIR:-$HOME/.config/modelswitch}"
ARCHIVE_DIR="$DATA_DIR/archive"
RETENTION_DAYS="${LOG_RETENTION_DAYS:-30}"
ARCHIVE_RETENTION_DAYS="${ARCHIVE_RETENTION_DAYS:-180}"

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
if [[ ! -d "$DATA_DIR" ]]; then
    die "Data directory does not exist: $DATA_DIR"
fi

mkdir -p "$ARCHIVE_DIR"

TIMESTAMP=$(date +"%Y%m%d-%H%M%S")

info "Starting log archiving"
info "Data dir:    $DATA_DIR"
info "Archive dir: $ARCHIVE_DIR"
info "Retention:   ${RETENTION_DAYS} days (archive), ${ARCHIVE_RETENTION_DAYS} days (cleanup)"

# ── Archive old NDJSON log files ─────────────────────────────────────────
# Patterns: logs.ndjson, logs.ndjson.1, logs.ndjson.2, etc.
#           audit.ndjson, audit.ndjson.1, etc.

LOG_PATTERNS=(
    "logs.ndjson"
    "logs.ndjson.*"
    "audit.ndjson"
    "audit.ndjson.*"
)

cd "$DATA_DIR"

ARCHIVED_COUNT=0
SKIP_COUNT=0

for pattern in "${LOG_PATTERNS[@]}"; do
    for filepath in $pattern; do
        # Skip if glob didn't match
        [[ -e "$filepath" ]] || continue

        # Check file age
        if [[ "$(uname)" == "Darwin" ]]; then
            # macOS: stat -f %m gives mtime epoch
            FILE_MTIME=$(stat -f %m "$filepath" 2>/dev/null || echo 0)
        else
            # Linux: stat -c %Y gives mtime epoch
            FILE_MTIME=$(stat -c %Y "$filepath" 2>/dev/null || echo 0)
        fi

        NOW=$(date +%s)
        AGE_DAYS=$(( (NOW - FILE_MTIME) / 86400 ))

        if [[ $AGE_DAYS -lt $RETENTION_DAYS ]]; then
            SKIP_COUNT=$((SKIP_COUNT + 1))
            continue
        fi

        # Compress and move to archive
        BASENAME=$(basename "$filepath")
        ARCHIVE_NAME="${BASENAME}.${TIMESTAMP}.gz"
        ARCHIVE_PATH="$ARCHIVE_DIR/$ARCHIVE_NAME"

        info "Archiving: $filepath (${AGE_DAYS}d old) -> $ARCHIVE_NAME"

        if gzip -c "$filepath" > "$ARCHIVE_PATH"; then
            # Verify the compressed archive
            if gzip -t "$ARCHIVE_PATH" 2>/dev/null; then
                # Remove the original file after successful compression
                rm -f "$filepath"
                ARCHIVED_COUNT=$((ARCHIVED_COUNT + 1))
                success "Archived: $ARCHIVE_NAME"
            else
                warn "Archive verification failed for $ARCHIVE_NAME, keeping original"
                rm -f "$ARCHIVE_PATH"
            fi
        else
            warn "Failed to compress $filepath"
        fi
    done
done

# ── Clean up old archives ────────────────────────────────────────────────
CLEANED_COUNT=0
if [[ "$ARCHIVE_RETENTION_DAYS" -gt 0 ]]; then
    info "Cleaning up archives older than ${ARCHIVE_RETENTION_DAYS} days..."

    while IFS= read -r -d '' old_archive; do
        rm -f "$old_archive"
        info "Deleted old archive: $(basename "$old_archive")"
        CLEANED_COUNT=$((CLEANED_COUNT + 1))
    done < <(find "$ARCHIVE_DIR" -name "*.gz" -mtime +"$ARCHIVE_RETENTION_DAYS" -print0 2>/dev/null)
fi

# ── Summary ──────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN} Log archiving complete${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo "Archived files:  $ARCHIVED_COUNT"
echo "Skipped (recent): $SKIP_COUNT"
echo "Cleaned archives: $CLEANED_COUNT"
echo "Archive dir:      $ARCHIVE_DIR"

# ── Log to archive log file ──────────────────────────────────────────────
LOG_FILE="$ARCHIVE_DIR/archive.log"
LOG_TIMESTAMP=$(date +"%Y-%m-%d %H:%M:%S")
echo "[$LOG_TIMESTAMP] archived=$ARCHIVED_COUNT skipped=$SKIP_COUNT cleaned=$CLEANED_COUNT" >> "$LOG_FILE"
