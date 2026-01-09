#!/usr/bin/env bash
#####################################################################
# QUICK TEST RUNNER FOR ROUTIIUM
#####################################################################
#
# This script runs tests against an already-running routiium server.
# Use this when you want to run tests without starting/stopping the server.
#
# Usage:
#   ./run_tests.sh              # Run all tests
#   ./run_tests.sh -v           # Verbose output
#   ./run_tests.sh -k test_name # Run specific test
#
#####################################################################

set -e  # Exit on error
set -u  # Exit on undefined variable

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="$PROJECT_ROOT/.env"

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if .env file exists
if [ ! -f "$ENV_FILE" ]; then
    log_error ".env file not found at $ENV_FILE"
    exit 1
fi

# Check if server is running
ROUTIIUM_BASE=$(grep ROUTIIUM_BASE "$ENV_FILE" | cut -d= -f2)
if [ -z "$ROUTIIUM_BASE" ]; then
    ROUTIIUM_BASE="http://127.0.0.1:8099"
fi

if [ "${ROUTIIUM_TEST_USE_EXTERNAL:-}" = "1" ]; then
    log_info "Checking if routiium server is running at $ROUTIIUM_BASE..."
    if ! curl -s "${ROUTIIUM_BASE}/status" > /dev/null 2>&1; then
        log_error "routiium server is not running at $ROUTIIUM_BASE"
        log_info "Start the server first with: cd .. && cargo run --release"
        exit 1
    fi
    log_success "Server is running"
else
    log_info "pytest will manage routiium server startup (set ROUTIIUM_TEST_USE_EXTERNAL=1 to skip)"
fi

# Setup virtual environment if needed
cd "$SCRIPT_DIR"

# Select system python binary for venv creation fallback
PYTHON_BIN=""
if command -v python >/dev/null 2>&1; then
    PYTHON_BIN="python"
elif command -v python3 >/dev/null 2>&1; then
    PYTHON_BIN="python3"
else
    log_error "Python not found (python/python3 missing from PATH)"
    exit 1
fi

# Ensure venv is valid (recreate if broken)
if [ -d ".venv" ]; then
    if [ ! -x ".venv/bin/python3" ] && [ ! -x ".venv/bin/python" ]; then
        log_warning "Existing .venv is missing Python; recreating..."
        rm -rf .venv
    fi
fi

if [ ! -d ".venv" ]; then
    log_info "Creating virtual environment..."
    if command -v uv >/dev/null 2>&1; then
        uv venv
    else
        "$PYTHON_BIN" -m venv .venv
    fi
fi

unset VIRTUAL_ENV
source .venv/bin/activate

VENV_PY=""
if [ -x ".venv/bin/python3" ]; then
    VENV_PY=".venv/bin/python3"
elif [ -x ".venv/bin/python" ]; then
    VENV_PY=".venv/bin/python"
else
    log_error "Virtual environment python not found in .venv/bin"
    exit 1
fi

# Ensure dependencies are installed
if ! "$VENV_PY" - <<'PY'
import importlib, sys
missing = []
for mod in ("requests", "pytest", "openai", "dotenv"):
    try:
        importlib.import_module(mod)
    except Exception:
        missing.append(mod)
if missing:
    sys.exit(1)
PY
then
    log_info "Installing dependencies..."
    if command -v uv >/dev/null 2>&1; then
        uv pip install --python "$VENV_PY" -e .
    else
        "$VENV_PY" -m pip install -e .
    fi
fi

# Load environment variables
export $(grep -v '^#' "$ENV_FILE" | xargs)

# Run pytest with passed arguments
log_info "Running tests..."
echo ""

if pytest tests/ "$@"; then
    echo ""
    log_success "====================================="
    log_success "All tests passed!"
    log_success "====================================="
    exit 0
else
    echo ""
    log_error "====================================="
    log_error "Some tests failed"
    log_error "====================================="
    exit 1
fi
