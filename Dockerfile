FROM rust@sha256:ce3708cb3672e2565ced6fa7c9f9102a36da1dada6aa5444324cd361a1326f98
RUN rustup component add clippy
RUN rustup component add rustfmt