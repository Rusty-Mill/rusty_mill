# Multi-stage build: compile in a full Rust image, ship just the binary in
# a minimal runtime image. Requires network access at build time to fetch
# rusty_stream's pinned git dependencies (rusty_tokio, rusty_wire, and
# their own transitive git deps) -- see Cargo.toml's comments for why
# those are pinned revs, not crates.io releases.
#
# Not verified with a real `docker build`/`docker run` as of this pass --
# no Docker daemon was available in the environment that wrote it. Stated
# plainly rather than silently assumed working; verify before relying on
# it for a real deployment.
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin rusty_stream

FROM debian:bookworm-slim
# io_uring is a host-kernel feature, not something this image provides --
# the container's host must run a kernel new enough for the io_uring ops
# this binary uses (realistically 5.11+), same as any other io_uring-based
# deployment. A container runtime that intercepts/restricts io_uring
# syscalls (some sandboxed runtimes do, for security reasons) will need an
# explicit allowance for this binary to start at all.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/rusty_stream /usr/local/bin/rusty_stream

ENV RUSTY_STREAM_ADDR=0.0.0.0:7420
ENV RUSTY_STREAM_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 7420

ENTRYPOINT ["/usr/local/bin/rusty_stream"]
