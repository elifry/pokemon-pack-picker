# Card recognition setup (optional)

Card recognition is an **optional** feature. When enabled, a camera button appears on each slot when you open a pack. You can point your camera at a card, capture it, and the app will call a **local** recognition service to identify the card (e.g. Pokemon TCG API card id). The result is stored so you can view and edit it later (name, holo status, regenerate image) without sending images to any third party after the initial scan.

The feature is **off by default**. Enable **Enable card recognition** in Settings to show the camera buttons; disable it to hide them.

## Requirements

- A local HTTP service that accepts a card image and returns a card identifier (e.g. Pokemon TCG API card id like `swsh12-123`).
- The app expects the service to expose a **POST** endpoint that accepts multipart form data with an `image` field (JPEG/PNG) and returns JSON such as: `{"cardId": "swsh12-123"}` (or `{"card_id": "swsh12-123"}`).

## Self-hosted recognition options

Pack Picker does **not** require a specific service or vendor. Any HTTP service that implements the contract above (POST with `image`, respond with `{"cardId": "..."}` or `{"card_id": "..."}`) will work. You can run your own pipeline or use any compatible implementation.

You can even use other services and wrap them in your own HTTP API to use this specific format.

A practical self-hosted approach is a pipeline that:

1. **Detects** the card in the image (e.g. YOLO).
2. **Identifies** the card (e.g. ResNet embedding vs a database of official card images).
3. Returns the **card id** used by [Pokemon TCG API](https://docs.pokemontcg.io/) (e.g. `base1-4`).

**One optional example** (not required): [lo-calvin/Trading-Card-Scanner](https://github.com/lo-calvin/Trading-Card-Scanner) (Python, YOLOv11 + ResNet18). It is a Streamlit app, not an HTTP API, so this repo includes a **Trading-Card-Scanner–specific wrapper** in **tools/trading-card-scanner-api/**.

### Steps for the bundled wrapper

1. **Clone Trading-Card-Scanner** (if not already under `tools/`):
   ```bash
   git clone https://github.com/lo-calvin/Trading-Card-Scanner.git tools/Trading-Card-Scanner
   ```
2. **Install dependencies** (from `tools/Trading-Card-Scanner`):
   ```bash
   cd tools/Trading-Card-Scanner && python3 -m venv venv && . venv/bin/activate
   pip install matplotlib_inline matplotlib ipython pokemontcgsdk ultralytics torchvision requests dotenv pandas imagehash streamlit flask
   ```
   (Omit the `logging` package from their `requirements.txt`—it conflicts with Python’s stdlib.)
3. **Model weights** (one-time): from the Trading-Card-Scanner repo root, run:
   ```bash
   python setup_weights.py
   ```
   This downloads pretrained YOLO11n-seg and ResNet18 weights and builds a **small** card-embedding dataset from the Pokemon TCG API (by default only the first ~80 cards from the API). Recognition will only identify cards in that set—or similar-looking ones—so “Card not found” is common until you expand the set. To embed more cards (recommended for real use), re-run with more pages, then restart the API:
   ```bash
   python setup_weights.py --pages 10 --max-cards 500
   ```
   Then restart `./tools/run-recognition-api.sh`. More cards = better recognition; you can tune `--pages` and `--max-cards` to balance coverage vs. build time and rate limits.
4. **Run the API**: from the pack-picker repo root, run `./tools/run-recognition-api.sh` (default port 5001). Or from `tools/Trading-Card-Scanner`: `python ../trading-card-scanner-api/server.py . --port 5001`.
5. **Configure Pack Picker**: in Settings, set **Recognition service URL** to `http://127.0.0.1:5001` and save.

Full options and troubleshooting: **tools/trading-card-scanner-api/README.md**.

## Configuring the app

1. Start your recognition service (e.g. run `./tools/run-recognition-api.sh`; it uses port 5001 to avoid conflict with macOS AirPlay on 5000). Ensure model weights are set up first (see **Steps for the bundled wrapper** above).
2. In **Settings**, check **Enable card recognition** (off by default).
3. Set **Recognition service URL** to the base URL of your service (e.g. `http://127.0.0.1:5001`). The app will POST to `{url}/recognize` with the image. Save settings.

If the URL is left empty, the feature is effectively off: when you tap the camera button, the app will show a message that recognition isn’t set up and offer to disable the feature or keep it on.

### Why "Card not found" or wrong card?

The bundled scanner (Trading-Card-Scanner) is **not** trained on the full TCG catalog. By default it only embeds ~80 cards from the first page of the Pokemon TCG API. So:

- It can only recognize cards that are in its embedding set (or ones that look very similar).
- If your card isn't in that set, you'll get "Card not found" or occasionally a wrong (but similar-looking) card.

**Fix:** Re-run `setup_weights.py` with more cards (e.g. `python setup_weights.py --pages 10 --max-cards 500` from the Trading-Card-Scanner repo root), then restart the recognition API. See **tools/trading-card-scanner-api/README.md** for details.

## Privacy

- Images are sent only to the **URL you configure** (your own machine or network). No images are sent to the Pokemon Pack Picker authors or to Pokemon TCG API for recognition; the app only uses the Pokemon TCG API to **fetch card details** (name, set, image) after your local service returns a card id.
