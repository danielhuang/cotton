FROM rust@sha256:969ca542302e38158733fdcb9ff465541391450691817ec011bb2fdffc3f64a8
RUN rustup component add clippy
RUN rustup component add rustfmt