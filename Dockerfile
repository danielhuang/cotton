FROM rust@sha256:5777f201f507075309c4d2d1c1e8d8219e654ae1de154c844341050016a64a0c
RUN rustup component add clippy
RUN rustup component add rustfmt