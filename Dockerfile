FROM rust@sha256:4da0fa233b5f91bb0edcc2a47906309aa94275a27de0d8b73b64e555de28748f
RUN rustup component add clippy
RUN rustup component add rustfmt