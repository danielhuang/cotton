FROM rust@sha256:e3d323070420270149fe65054f65bf680d7ddb3d66008a0549e6afe6b320c8eb
RUN rustup component add clippy
RUN rustup component add rustfmt