"""The engine's scores against the reference's: per position, the largest difference, whether the likeliest token is
the same, and the KL divergence of the engine's probabilities from the reference's.

    python compare.py REF_PREFIX MODEL.ccm RUN_PROGRAM

REF_PREFIX is what reference.py wrote (its .ids.json and .logits.f32); RUN_PROGRAM the engine's run example
(target/release/examples/run), which feeds the same tokens one at a time and writes its scores.
"""
import json
import os
import subprocess
import sys
import tempfile

import numpy as np

ref, model, run = sys.argv[1], sys.argv[2], sys.argv[3]
ids = json.load(open(ref + ".ids.json"))
want = np.fromfile(ref + ".logits.f32", dtype=np.float32).reshape(len(ids), -1)
with tempfile.TemporaryDirectory() as tmp:
    ours = os.path.join(tmp, "ours.f32")
    subprocess.run([run, model, "--ids", ",".join(map(str, ids)), "--logits", ours], check=True)
    got = np.fromfile(ours, dtype=np.float32).reshape(len(ids), -1)
got = got[:, :want.shape[1]]
def softmax(x):
    e = np.exp(x - x.max(-1, keepdims=True))
    return e / e.sum(-1, keepdims=True)
p, q = softmax(want.astype(np.float64)), softmax(got.astype(np.float64))
kl = (p * (np.log(p + 1e-30) - np.log(q + 1e-30))).sum(-1)
same = (want.argmax(-1) == got.argmax(-1))
print(f"{len(ids)} positions: largest difference {np.abs(want - got).max():.4f} (scores up to {np.abs(want).max():.1f}); "
      f"same top token {same.sum()}/{len(ids)}; KL mean {kl.mean():.6f} max {kl.max():.6f}")
