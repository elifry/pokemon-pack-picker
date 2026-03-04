# Trading-Card-Scanner API wrapper

This folder is a **single optional option**: an HTTP server that wraps [lo-calvin/Trading-Card-Scanner](https://github.com/lo-calvin/Trading-Card-Scanner) so it exposes **POST /recognize** (image in → `{"cardId": "setX-nnn"}` out). Pokemon Pack Picker (and any other client) can use **any** recognition service that implements that contract; this wrapper is only for people who choose to use Trading-Card-Scanner.

You cannot use Trading-Card-Scanner “directly” as an API — that repo is a Streamlit app (scan from camera/file, add to collection). This wrapper runs their recognition pipeline and exposes the endpoint.

## Steps

1. **Clone and set up Trading-Card-Scanner** (one-time):

   ```bash
   git clone https://github.com/lo-calvin/Trading-Card-Scanner.git
   cd Trading-Card-Scanner
   python3 -m venv venv
   source venv/bin/activate   # or: venv\Scripts\activate on Windows
   pip install -r requirements.txt
   ```

   **Model weights:** The upstream repo does not ship all weights. From the Trading-Card-Scanner repo root, run the one-time setup script to download or generate them:
   ```bash
   cd /path/to/Trading-Card-Scanner
   . venv/bin/activate
   python setup_weights.py
   ```
   This creates:
   - `res/detection_weights/yolo11n_seg_best_10epochs.pt` (pretrained YOLO11n-seg; or use your own card-trained model)
   - `res/detection_weights/resnet18_embeddings.pth` (ResNet18 backbone from torchvision)
   - `res/classification_embeddings/Resnet18_embeddings.pt` (card embeddings built from Pokemon TCG API images)

   **Important:** By default the script embeds only **~80 cards** (first API page). Recognition will only identify cards in that set. To recognize more cards, re-run with more pages and restart the API:
   ```bash
   python setup_weights.py --pages 10 --max-cards 500
   ```
   Then restart the recognition API. More cards = better recognition; tune `--pages` and `--max-cards` to balance coverage vs. build time and rate limits.

   If weights are missing, the API server still starts; `GET /` returns `model_loaded: false` and `POST /recognize` returns 503 until the files are in place.

2. **Install this wrapper’s dependency** (in the same venv or a dedicated one):

   ```bash
   pip install -r /path/to/pokemon-pack-picker/tools/trading-card-scanner-api/requirements.txt
   ```

3. **Run the API**. From the **pokemon-pack-picker** repo root you can use the helper script:

   ```bash
   ./tools/run-recognition-api.sh
   ```

   Or from the Trading-Card-Scanner repo root:

   ```bash
   cd /path/to/Trading-Card-Scanner
   python /path/to/pokemon-pack-picker/tools/trading-card-scanner-api/server.py
   ```

   Or set the repo path explicitly:

   ```bash
   export TRADING_CARD_SCANNER_DIR=/path/to/Trading-Card-Scanner
   python /path/to/pokemon-pack-picker/tools/trading-card-scanner-api/server.py
   ```

   By default the run script uses port **5001** (5000 is often taken by macOS AirPlay Receiver). Use `--port` or set `PORT=5000` when running the script. The server binds to `127.0.0.1`.

4. **Configure your app**: In Pack Picker Settings (or whatever client you use), set the recognition service URL to `http://127.0.0.1:5001` (or the port you use). The client will POST to `{url}/recognize` with the captured image.

## API contract (same as any compatible recognition service)

- **POST /recognize**  
  - Body: multipart form data, field name **image** (JPEG or PNG).  
  - Success: `200` with JSON `{"cardId": "setX-nnn"}`.  
  - Errors: `400` (missing/empty image), `422` (no cards detected), `500` (server error).

## Notes

- Trading-Card-Scanner uses Windows-style paths in `src/model.py` and `src/retriever.py`. On Linux/macOS, if you get file-not-found errors for `res/` assets, you may need to change those paths to use `os.path.join` or forward slashes.
- Their pipeline detects one or more cards in the image; this wrapper returns the **first** detected card id.
