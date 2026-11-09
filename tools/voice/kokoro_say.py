"""Kokoro-82M (hexgrad, Apache-2.0) saying lines for tools/voice/tutorial.py, run with the Python of
a venv that has Kokoro (it needs a newer numpy than Chatterbox's, so it lives apart): one JSON
request a line on stdin, {"text": ..., "out": "<file.wav>"}, answered "ok" (or "error: ...") on
stdout once the 24 kHz WAV is written. The voice is the first argument (af_heart: "Heart").

Setup (Python 3.13), in a folder of its own:
    python -m venv kokoro_venv
    kokoro_venv/Scripts/python -m pip install torch --index-url https://download.pytorch.org/whl/cpu
    kokoro_venv/Scripts/python -m pip install --only-binary=:all: "spacy>=3.8.5" numpy scipy loguru huggingface-hub transformers num2words spacy-curated-transformers phonemizer-fork espeakng-loader soundfile addict
    kokoro_venv/Scripts/python -m pip install --no-deps --ignore-requires-python kokoro==0.9.4 misaki==0.9.4
(kokoro 0.9.4 says Python below 3.13 and pins numpy 1.26.4, which has no 3.13 build; it runs on
3.13 with a current numpy.) The model and the voice are fetched from Hugging Face on first use.
"""
import json
import sys

import numpy as np
import soundfile as sf
from kokoro import KPipeline

RATE = 24_000

voice = sys.argv[1] if len(sys.argv) > 1 else "af_heart"
# (American English: the voice's own; "b" would be British.)
pipeline = KPipeline(lang_code=voice[0], repo_id="hexgrad/Kokoro-82M")
print("ready", flush=True)
for request in sys.stdin:
    try:
        job = json.loads(request)
        # The line as one passage: Kokoro paces its sentences itself.
        parts = [np.asarray(audio, dtype=np.float32) for _, _, audio in pipeline(job["text"], voice=voice, speed=1.0)]
        sf.write(job["out"], np.concatenate(parts) if parts else np.zeros(RATE // 10, np.float32), RATE)
        print("ok", flush=True)
    except Exception as e:  # (the caller treats it as a failed take)
        print(f"error: {e}".replace("\n", " "), flush=True)
