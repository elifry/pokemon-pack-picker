# Card recognition setup (optional)

Card recognition is an **optional** feature. When enabled, a camera button appears on each slot when you open a pack. You can point your camera at a card, capture it, and the app will call a **local** recognition service to identify the card (e.g. Pokemon TCG API card id). The result is stored so you can view and edit it later (name, holo status, regenerate image) without sending images to any third party after the initial scan.

You can use the app without this feature: disable **Enable card recognition** in Settings and the camera buttons will not appear.

## Requirements

- A local HTTP service that accepts a card image and returns a card identifier (e.g. Pokemon TCG API card id like `swsh12-123`).
- The app expects the service to expose a **POST** endpoint that accepts multipart form data with an `image` field (JPEG/PNG) and returns JSON such as: `{"cardId": "swsh12-123"}` (or `{"card_id": "swsh12-123"}`).

## Self-hosted recognition options

A practical self-hosted option is to run a recognition pipeline that:

1. **Detects** the card in the image (e.g. YOLO).
2. **Identifies** the card (e.g. ResNet50 embedding vs a database of official card images).
3. Returns the **card id** used by [Pokemon TCG API](https://docs.pokemontcg.io/) (e.g. `base1-4`).

Example projects you can run locally and point the app at:

- **[lo-calvin/Trading-Card-Scanner](https://github.com/lo-calvin/Trading-Card-Scanner)** — Python, YOLOv11 for detection and ResNet50 for identification; then use Pokemon TCG API for details. You would expose a small HTTP endpoint (e.g. Flask/FastAPI) that accepts an image, runs the pipeline, and returns `{"cardId": "..."}`.

## Configuring the app

1. Start your recognition service (e.g. on `http://127.0.0.1:5000`).
2. In **Settings**, ensure **Enable card recognition** is checked.
3. Set **Recognition service URL** to the base URL of your service (e.g. `http://127.0.0.1:5000`). The app will POST to `{url}/recognize` with the image.

If the URL is left empty, the feature is effectively off: when you tap the camera button, the app will show a message that recognition isn’t set up and offer to disable the feature or keep it on.

## Privacy

- Images are sent only to the **URL you configure** (your own machine or network). No images are sent to the Pokemon Pack Picker authors or to Pokemon TCG API for recognition; the app only uses the Pokemon TCG API to **fetch card details** (name, set, image) after your local service returns a card id.
