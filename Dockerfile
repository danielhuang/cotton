FROM rust@sha256:b7f46daf042e98e8b49921705a1deebbfc69f59879a9438fbce562dce1873ce8
RUN rustup component add clippy
RUN rustup component add rustfmt