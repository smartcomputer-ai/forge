FROM node:24.13.0-bookworm-slim@sha256:46feb5752989c05b8606e6323fbbc3db667d14ade1c24f5d0d44d9ca9909d607 AS node
FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97

COPY --from=node /usr/local/ /usr/local/
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        binutils ca-certificates clang cmake git libprotobuf-dev libssl-dev musl-tools pkg-config protobuf-compiler \
    && test -f /usr/include/google/protobuf/duration.proto \
    && rm -rf /var/lib/apt/lists/*
# The environment daemon ships as a static musl binary so it runs on any
# Linux image, whatever glibc it carries; musl-tools provides the C
# toolchain aws-lc-rs compiles with for that target.
RUN rustup target add x86_64-unknown-linux-musl
RUN git config --system --add safe.directory /workspace

ENV PROTOC_INCLUDE=/usr/include

WORKDIR /workspace
