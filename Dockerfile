FROM rust@sha256:6052afe7c422c163798bb9064b7215db15c5f790214ee2c2e787daf8ed3de92a
RUN rustup component add clippy
RUN rustup component add rustfmt