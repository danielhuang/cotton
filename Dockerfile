FROM rust@sha256:ec7dae306d01d4c52d2b6cce4a62a8da2f2e54df543e527e1656ae7c4ef632b3
RUN rustup component add clippy
RUN rustup component add rustfmt