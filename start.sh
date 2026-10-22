#!/usr/bin/env bash
# Richtet beim ersten Start alles ein und öffnet die Weboberfläche.
set -e
cd "$(dirname "$0")"
PY=${PYTHON:-}
if [ -z "$PY" ]; then
  for c in python3.11 python3.10 python3.12 python3; do command -v $c >/dev/null && PY=$c && break; done
fi
if [ ! -d .venv ]; then
  echo "Erstelle Umgebung mit $PY …"
  $PY -m venv .venv
  .venv/bin/pip install --upgrade pip
  .venv/bin/pip install -r requirements.txt
  # TensorFlow wird nicht gebraucht (ONNX reicht) und kollidiert mit numpy 2
  .venv/bin/pip uninstall -y tensorflow tensorflow-estimator keras >/dev/null 2>&1 || true
  read -p "Spurtrennung (Gesang/Bass/…) installieren? ~1 GB [j/N] " a
  if [ "$a" = "j" ] || [ "$a" = "J" ]; then
    .venv/bin/pip install -r requirements-stems.txt --extra-index-url https://download.pytorch.org/whl/cpu
  fi
fi
command -v ffmpeg >/dev/null || echo "WARNUNG: ffmpeg fehlt (macOS: brew install ffmpeg, Linux: sudo apt install ffmpeg)"
exec .venv/bin/python app.py "$@"
