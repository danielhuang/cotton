FROM rust@sha256:491c4b70ec4c86a5a32a76b35c42f1924745d7587c79a39b9534330a1e304e71
RUN rustup component add clippy
RUN rustup component add rustfmt