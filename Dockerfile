FROM rust@sha256:28ee8822965a932e229599b59928f8c2655b2a198af30568acf63e8aff0e8a3a
RUN rustup component add clippy
RUN rustup component add rustfmt