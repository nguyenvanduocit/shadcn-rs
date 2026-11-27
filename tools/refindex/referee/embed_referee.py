# /// script
# requires-python = ">=3.12,<3.13"
# dependencies = ["sentence-transformers>=5", "numpy"]
# ///
"""Independent referee for refx embeddings.

Computes the same model's sentence embeddings with the reference implementation
(sentence-transformers / PyTorch) and compares them with `refx vectors`. A port bug in the
candle path (wrong pooling, missing final norm, wrong local-attention window, mis-mapped
weights) shows up as cosine well below 1. Run:

    uv run tools/refindex/referee/embed_referee.py /path/to/refx

`--broken` feeds refx a perturbed text to show the check fails when the vectors differ.
"""

import json
import subprocess
import sys

import numpy as np
from sentence_transformers import SentenceTransformer

MODEL = "ibm-granite/granite-embedding-small-english-r2"
TEXTS = [
    "taffy/src/compute/flexbox.rs\nfunction_item collect_flex_lines\n/// Collect items into flex lines\nfn collect_flex_lines() {}",
    "how are flex items broken into lines",
    "propagate dirty flag to subscribers when a signal changes",
    "blink::LayoutBox::Paint(const PaintInfo& info) const { DoThing(); }",
    # long enough to exercise truncation at 256 tokens and the 128-token local attention window
    " ".join(f"token{i} computeLayout(node_{i});" for i in range(400)),
]


def main():
    refx = sys.argv[1]
    broken = "--broken" in sys.argv
    m = SentenceTransformer(MODEL, device="cpu", model_kwargs={"dtype": "float32"})
    m.max_seq_length = 256
    ref = m.encode(TEXTS, normalize_embeddings=True)
    sent = [t + " unrelated words about cooking pasta" if broken and i == 1 else t
            for i, t in enumerate(TEXTS)]
    out = subprocess.run([refx, "vectors", *sent], capture_output=True, text=True, check=True)
    got = np.array(json.loads(out.stdout), dtype=np.float32)
    cos = (ref * got).sum(1) / np.linalg.norm(got, axis=1)
    ok = True
    for t, c in zip(TEXTS, cos):
        flag = "ok" if c > 0.999 else "MISMATCH"
        ok &= c > 0.999
        print(f"{c:.5f} {flag}  {t[:60]!r}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
