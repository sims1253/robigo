# robigo-oracle:4.5.0: CI's oracle platform (.github/actions/setup) as an
# image, with the system libraries of the package suite. Built on demand by
# scripts/package-suite.py; also what scripts/cran-gate.py `record` runs in.
#   docker build -t robigo-oracle:4.5.0 tests/packages
FROM debian:trixie
# the definition's hash (scripts/package-suite.py rebuilds when it changes)
ARG ORACLE_ID=
LABEL robigo.oracle=$ORACLE_ID
COPY apt.txt /tmp/apt.txt
RUN export DEBIAN_FRONTEND=noninteractive \
 && apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends r-base-core=4.5.0-3 r-base-dev \
      $(sed -e 's/#.*//' /tmp/apt.txt) \
 && rm -rf /var/lib/apt/lists/* \
 && R --version > /dev/null
