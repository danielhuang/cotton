#!/bin/bash
set -euo pipefail

RUSTFLAGS="-C target-feature=+crt-static" cargo build --target x86_64-unknown-linux-gnu
podman run -it -v ./target/x86_64-unknown-linux-gnu/debug/cotton:/usr/bin/cotton node /bin/bash -c "mkdir /app && cd /app && bash"
