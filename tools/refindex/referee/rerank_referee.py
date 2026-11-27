# /// script
# requires-python = ">=3.12,<3.13"
# dependencies = ["sentence-transformers>=5", "numpy"]
# ///
"""Independent referee for the refx cross-encoder.

Scores the same (query, doc) pairs with sentence-transformers' CrossEncoder (the reference
implementation of granite-embedding-reranker-english-r2, sigmoid activation) and compares
with `refx scores`. A wrong head (GELU instead of SiLU, a softmax over one label, a missing
norm, mis-mapped weights) shows up as a score difference or a different order.

    uv run tools/refindex/referee/rerank_referee.py /path/to/refx [--broken]

`--broken` swaps two docs for refx only, so the check must fail.
"""

import json
import subprocess
import sys

import numpy as np
from sentence_transformers import CrossEncoder

MODEL = "ibm-granite/granite-embedding-reranker-english-r2"
QUERY = "how are flex items broken into lines"
DOCS = [
    "references/taffy/src/compute/flexbox.rs\nfunction_item collect_flex_lines\n/// Collect the flex items into flex lines\nfn collect_flex_lines(...)",
    "references/taffy/src/style/grid.rs\nstruct_item GridTrack\npub struct GridTrack { base_size: f32 }",
    "references/chromium/cc/trees/layer_tree_host.cc\nfunction_definition LayerTreeHost::Commit\nvoid LayerTreeHost::Commit() {}",
    "docs/adr/0014-layout-runs-on-a-retained-tree-in-parallel-islands.md\ndoc Flex lines\nA flex container breaks its items into lines when wrap is set.",
]


def main():
    refx = sys.argv[1]
    broken = "--broken" in sys.argv
    ref = CrossEncoder(MODEL, device="cpu", max_length=384,
                       model_kwargs={"dtype": "float32"}).predict([(QUERY, d) for d in DOCS])
    sent = [DOCS[1], DOCS[0], *DOCS[2:]] if broken else DOCS
    out = subprocess.run([refx, "scores", QUERY, *sent], capture_output=True, text=True, check=True)
    got = np.array(json.loads(out.stdout))
    ok = True
    for d, r, g in zip(DOCS, ref, got):
        good = abs(r - g) < 0.01
        ok &= good
        print(f"ref {r:.4f}  refx {g:.4f}  {'ok' if good else 'MISMATCH'}  {d.splitlines()[0]}")
    ok &= list(np.argsort(-ref)) == list(np.argsort(-got))
    print("order", "same" if list(np.argsort(-ref)) == list(np.argsort(-got)) else "DIFFERENT")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
