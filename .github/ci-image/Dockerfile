# The CI image: what .github/actions/setup and .github/actions/package-suite
# install, baked in once instead of in every job (docs/verification.md, CI).
# scripts/ci-image.sh tags each target by the hash of this directory, the
# package suite's apt.txt and rust-toolchain.toml, and builds and pushes a
# tag only when it is missing, so a change here is tested by the pull
# request that makes it.
#
#   base      GNU R 4.5.0 and the harnesses' tools (corpora, fuzz, cran, perf)
#   rust      base + the pinned Rust toolchain (lint, build, test)
#   packages  base + the package suite's system libraries (smoke, package-gate)
FROM debian:trixie AS base
ARG DEBIAN_FRONTEND=noninteractive
LABEL org.opencontainers.image.source=https://github.com/sims1253/robigo
LABEL org.opencontainers.image.description="robigo CI: Debian trixie with GNU R 4.5.0"
COPY apt.txt /tmp/robigo-ci/apt.txt
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends $(sed -e 's/#.*//' /tmp/robigo-ci/apt.txt) > /dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && R --version | head -1
ENV R_HOME=/usr/lib/R
# .github/actions/setup skips its installs when this file exists.
RUN echo base > /etc/robigo-ci-image

FROM base AS rust
# Outside $HOME: GitHub's container jobs set HOME=/github/home.
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
COPY rust-toolchain.toml /tmp/robigo-ci/rust-toolchain.toml
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none \
 && cd /tmp/robigo-ci && rustup toolchain install \
 && rustup default "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" \
 && rm -rf "$CARGO_HOME/registry" \
 && rustc --version && cargo clippy --version && cargo fmt --version \
 && echo rust > /etc/robigo-ci-image

FROM base AS packages
ARG DEBIAN_FRONTEND=noninteractive
COPY packages-apt.txt /tmp/robigo-ci/packages-apt.txt
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends $(sed -e 's/#.*//' /tmp/robigo-ci/packages-apt.txt) > /dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && echo packages > /etc/robigo-ci-image
