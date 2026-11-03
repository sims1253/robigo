# robigo-oracle:4.5.0: CI's oracle platform (.github/actions/setup) as an
# image, with the system libraries of the package suite. Built on demand by
# scripts/package-suite.py; also what scripts/cran-gate.py `record` runs in.
#   docker build -t robigo-oracle:4.5.0 tests/packages
FROM debian:trixie
ENV DEBIAN_FRONTEND=noninteractive
COPY apt.txt /tmp/apt.txt
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends r-base-core=4.5.0-3 r-base-dev \
      $(grep -v '^#' /tmp/apt.txt) \
 && R --version | head -1
