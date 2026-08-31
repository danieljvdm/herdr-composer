#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
command -v cargo >/dev/null || { printf 'Building Herdr Composer requires Rust and Cargo.\n' >&2; exit 1; }
cargo build --manifest-path "$root/Cargo.toml" --target-dir "$root/target" --release --locked
mkdir -p "$root/bin"
# Replace atomically so an already open composer can keep running.
install -m 755 "$root/target/release/herdr-composer" "$root/bin/herdr-composer.new"
mv "$root/bin/herdr-composer.new" "$root/bin/herdr-composer"
