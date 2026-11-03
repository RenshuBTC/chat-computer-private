#!/bin/sh
# Builds the model engine's program for Chat Computer on Linux, one static file for each kind of processor that runs
# on any Linux (x86-64 for PCs; 64-bit ARM for Macs with Apple silicon, under Fedora Asahi Remix), and puts them in
# chatcomputer/brain/bin/, where Chat Computer takes them from (the computers it's installed on have no Rust
# compiler; that folder is part of what an update brings).
# Reproducible: the paths it was built in are left out of the files, so the same source builds the same files
# wherever it's built. Linked by the linker that comes with Rust (rust-lld), so nothing else is needed.
# With --check, builds them elsewhere and checks they're exactly the files there (as CI does).
set -eu
cd "$(dirname "$0")/.."
os=$PWD
bin="$os/../chatcomputer/brain/bin"
out=$bin
if [ "${1:-}" = "--check" ]; then
    out=$(mktemp -d)
fi
cargo_home=${CARGO_HOME:-$HOME/.cargo}
for arch in x86_64 aarch64; do
    target=$arch-unknown-linux-musl
    RUSTFLAGS="-C linker=rust-lld -C strip=symbols --remap-path-prefix=$os=/src --remap-path-prefix=$cargo_home=/cargo" \
        cargo build --release -p model --features std --bin chat-computer-model --target "$target"
    mkdir -p "$out"
    cp "target/$target/release/chat-computer-model" "$out/chat-computer-model-$arch"
    (cd "$out" && sha256sum "chat-computer-model-$arch" > "chat-computer-model-$arch.sha256")
done
if [ "$out" != "$bin" ]; then
    for arch in x86_64 aarch64; do
        cmp "$out/chat-computer-model-$arch" "$bin/chat-computer-model-$arch"
        (cd "$bin" && sha256sum -c "chat-computer-model-$arch.sha256")
    done
    rm -rf "$out"
    echo "The engine's files in chatcomputer/brain/bin are what this source builds."
else
    cat "$bin"/chat-computer-model-*.sha256
fi
