FROM rust@sha256:c09b1bb91bdc5b44a2f761333c13f9a0f56ef8a677391be117e749be0b7427e8
RUN rustup component add clippy
RUN rustup component add rustfmt