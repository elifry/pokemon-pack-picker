#!/usr/bin/env bash
# Start the Trading-Card-Scanner recognition API (for Pack Picker card recognition).
# Run from the pokemon-pack-picker repo root. Requires tools/Trading-Card-Scanner to be cloned.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCANNER_DIR="${SCRIPT_DIR}/Trading-Card-Scanner"
API_DIR="${SCRIPT_DIR}/trading-card-scanner-api"

if [[ ! -d "$SCANNER_DIR" ]]; then
  echo "Trading-Card-Scanner not found at $SCANNER_DIR"
  echo "Run: git clone https://github.com/lo-calvin/Trading-Card-Scanner.git tools/Trading-Card-Scanner"
  exit 1
fi

cd "$SCANNER_DIR" || exit 1
if [[ ! -d venv ]]; then
  echo "Creating venv and installing dependencies..."
  python3 -m venv venv
  . venv/bin/activate
  pip install matplotlib_inline matplotlib ipython pokemontcgsdk ultralytics torchvision requests dotenv pandas imagehash streamlit flask
else
  . venv/bin/activate
fi
# Use 5001 by default (macOS AirPlay Receiver often uses 5000)
exec python "$API_DIR/server.py" . --port "${PORT:-5001}"
