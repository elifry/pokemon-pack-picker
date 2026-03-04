#!/usr/bin/env python3
"""
Minimal HTTP API that wraps [Trading-Card-Scanner](https://github.com/lo-calvin/Trading-Card-Scanner)
so any client (e.g. Pokemon Pack Picker) can call a single POST /recognize endpoint.

Run from the Trading-Card-Scanner repo root (see README). Expects multipart form
field "image" (JPEG/PNG). Returns JSON: {"cardId": "setX-nnn"} or 4xx on failure.
"""

import os
import sys
import tempfile
import argparse

# Must run from Trading-Card-Scanner repo root so res/ and src/ resolve.
def main():
    parser = argparse.ArgumentParser(
        description="Trading-Card-Scanner API wrapper (POST /recognize)"
    )
    parser.add_argument(
        "scanner_dir",
        nargs="?",
        default=os.environ.get("TRADING_CARD_SCANNER_DIR"),
        help="Path to Trading-Card-Scanner repo root (or set TRADING_CARD_SCANNER_DIR)",
    )
    parser.add_argument("--host", default="127.0.0.1", help="Bind host (default 127.0.0.1)")
    parser.add_argument("--port", type=int, default=5001, help="Port (default 5001; 5000 often used by AirPlay)")
    args = parser.parse_args()
    if not args.scanner_dir or not os.path.isdir(args.scanner_dir):
        print(
            "Usage: run from Trading-Card-Scanner root or pass scanner_dir / set TRADING_CARD_SCANNER_DIR",
            file=sys.stderr,
        )
        sys.exit(1)
    os.chdir(args.scanner_dir)
    src = os.path.join(args.scanner_dir, "src")
    if src not in sys.path:
        sys.path.insert(0, src)
    run_app(host=args.host, port=args.port)


def run_app(host: str = "127.0.0.1", port: int = 5001) -> None:
    from flask import Flask, request, jsonify

    # Import after chdir so res/ and src/ resolve
    try:
        from model import Model
        model = Model()
        model_error = None
    except Exception as e:
        model = None
        model_error = str(e)
        print("Model failed to load (missing weights?):", model_error, file=sys.stderr)
        print("Put YOLO and ResNet18 weights in res/ (see tools/trading-card-scanner-api/README.md)", file=sys.stderr)

    app = Flask(__name__)

    @app.route("/recognize", methods=["POST"])
    def recognize():
        if model is None:
            return jsonify({"error": "model not loaded", "detail": model_error}), 503
        if "image" not in request.files:
            return jsonify({"error": "missing image field"}), 400
        f = request.files["image"]
        if not f.filename and not f.stream.read(1):
            f.stream.seek(0)
            return jsonify({"error": "empty image"}), 400
        f.stream.seek(0)
        suffix = ".png" if f.filename and f.filename.lower().endswith(".png") else ".jpg"
        try:
            with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as tmp:
                tmp.write(f.read())
                tmp_path = tmp.name
        except Exception as e:
            return jsonify({"error": str(e)}), 500
        try:
            model.process_image(tmp_path)
        except Exception as e:
            os.unlink(tmp_path)
            return jsonify({"error": str(e)}), 500
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)
        if not model.results:
            return jsonify({
                "error": "no cards detected",
                "detail": "A card may have been detected but could not be identified (no match in the embedding database or TCG API lookup failed).",
            }), 422
        first = next(iter(model.results.values()))
        card_id = getattr(first, "id", None)
        if not card_id:
            return jsonify({"error": "card id missing"}), 500
        return jsonify({"cardId": card_id})

    @app.route("/", methods=["GET"])
    def index():
        return jsonify({
            "service": "trading-card-scanner-api",
            "endpoint": "/recognize",
            "model_loaded": model is not None,
            **({"model_error": model_error} if model_error else {}),
        })

    app.run(host=host, port=port, threaded=True)


if __name__ == "__main__":
    main()
