FROM rust@sha256:cda91c602092bbcae9e39626dacbdc2c6b0df9b053b329dc43e8525d1369e57d
RUN rustup component add clippy
RUN rustup component add rustfmt