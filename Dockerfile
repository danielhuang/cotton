FROM rust@sha256:6a6dda669f020fa1fcb0903e37a049484fbf4b4699c8cb89db26ca030f475259
RUN rustup component add clippy
RUN rustup component add rustfmt