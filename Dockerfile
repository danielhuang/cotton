FROM rust@sha256:fb477b5dff4e71ed2f93c287926811bdffde1cfd84f67c06431ef2a884090543
RUN rustup component add clippy
RUN rustup component add rustfmt