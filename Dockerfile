FROM rust@sha256:c5f0f8ecc0e1a1e32e480afb56d1496f38bf337806200a236e405081685bef3f
RUN rustup component add clippy
RUN rustup component add rustfmt