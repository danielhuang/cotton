FROM rust@sha256:0dd183faf7bc5b9b8efe81cfd42701a5283577520b185b511e322e5bf52f8fc7
RUN rustup component add clippy
RUN rustup component add rustfmt