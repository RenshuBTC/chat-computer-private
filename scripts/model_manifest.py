"""Writes chatcomputer/brain/models.json: the models Chat Computer can think with, each pinned to one revision of
Qwen's own repository on Hugging Face, with every file needed (its size and SHA-256), so what's downloaded is
checked against what was pinned here, not against what a server says at the time.

    python3 scripts/model_manifest.py

Run again to move to a newer revision (change REVISIONS first). The files kept are the ones the converter reads:
the settings, the tokenizer, the chat template, and the weights (their shards hold the vision part too, which the
converter keeps). Each model says whether it sees ("sees": its config.json has a vision part the engine runs, as
os/model's convert.rs decides): a file of one that sees made without that part (before the engine kept it) is made
again by Chat Computer's apps.
"""
import json
import os
import sys
import urllib.request

# name: (label, repository, revision, the computer's memory it needs at least in GB, bits a weight)
REVISIONS = {
    "qwen3.8-27b": ("Qwen3.8 27B", "Qwen/Qwen3.8-27B", "1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0", 48, 4),
    "qwen3.6-35b-a3b": ("Qwen3.6 35B-A3B", "Qwen/Qwen3.6-35B-A3B", "995ad96eacd98c81ed38be0c5b274b04031597b0", 32, 4),
    "qwen3.5-9b": ("Qwen3.5 9B", "Qwen/Qwen3.5-9B", "c202236235762e1c871ad0ccb60c8ee5ba337b9a", 16, 4),
    "qwen3.5-4b": ("Qwen3.5 4B", "Qwen/Qwen3.5-4B", "851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a", 8, 4),
    "qwen3.5-2b": ("Qwen3.5 2B", "Qwen/Qwen3.5-2B", "15852e8c16360a2fea060d615a32b45270f8a8fc", 4, 4),
    "qwen3.5-0.8b": ("Qwen3.5 0.8B", "Qwen/Qwen3.5-0.8B", "2fc06364715b967f1860aea9cf38778875588b17", 0, 8),
}
KEEP = ("config.json", "tokenizer.json", "tokenizer_config.json", "chat_template.jinja", "generation_config.json")
OUT = os.path.join(os.path.dirname(__file__), "..", "chatcomputer", "brain", "models.json")


def files_of(repo: str, revision: str) -> list[dict]:
    url = f"https://huggingface.co/api/models/{repo}/revision/{revision}?blobs=true"
    with urllib.request.urlopen(url, timeout=60) as r:
        info = json.load(r)
    if info.get("sha") != revision:
        sys.exit(f"{repo}: asked for {revision}, got {info.get('sha')}")
    out = []
    for s in info["siblings"]:
        name = s["rfilename"]
        if name in KEEP or name.endswith(".safetensors"):
            lfs = s.get("lfs") or {}
            sha = lfs.get("sha256")
            if not sha:
                # A small file kept in git itself: its hash is worked out from its bytes here.
                import hashlib
                with urllib.request.urlopen(f"https://huggingface.co/{repo}/resolve/{revision}/{name}", timeout=60) as r:
                    data = r.read()
                sha = hashlib.sha256(data).hexdigest()
                size = len(data)
            else:
                size = lfs.get("size", s.get("size"))
            out.append({"name": name, "size": size, "sha256": sha})
    return sorted(out, key=lambda f: f["name"])


def sees(repo: str, revision: str) -> bool:
    """Whether the model has a vision part the engine runs: config.json's vision_config, without features taken
    from its middle into the language model's first layers (deepstack_visual_indexes), as convert.rs decides."""
    with urllib.request.urlopen(f"https://huggingface.co/{repo}/resolve/{revision}/config.json", timeout=60) as r:
        vision = json.load(r).get("vision_config")
    return bool(vision) and not vision.get("deepstack_visual_indexes")


def main() -> None:
    models = []
    for name, (label, repo, revision, memory, bits) in REVISIONS.items():
        files = files_of(repo, revision)
        weights = sum(f["size"] for f in files if f["name"].endswith(".safetensors"))
        print(f"{name}: {len(files)} files, {weights / 1e9:.1f} GB of weights")
        models.append({"name": name, "label": label, "repo": repo, "revision": revision, "memory_gb": memory,
                       "bits": bits, "sees": sees(repo, revision), "files": files})
    with open(OUT, "w") as f:
        json.dump({"models": models}, f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    main()
