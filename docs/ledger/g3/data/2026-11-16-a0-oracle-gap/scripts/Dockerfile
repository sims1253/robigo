# A0's image-toolchain cells: Debian trixie (gcc 14.2, glibc 2.41), the
# oracle image's platform, with GNU R 4.6.2 from the pinned tarball built
# with the given flags and configure options into /opt/R, plus valgrind
# and python3 for the measurements.
FROM debian:trixie
ARG DEBIAN_FRONTEND=noninteractive
ARG CFLAGS_A0
ARG FFLAGS_A0
ARG LDFLAGS_A0
ARG CONFIGURE_A0
COPY oracle-apt.txt R-4.6.2.tar.gz /tmp/a0/
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends $(sed -e 's/#.*//' /tmp/a0/oracle-apt.txt) valgrind > /dev/null \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; cd /tmp/a0 && tar -xzf R-4.6.2.tar.gz && cd R-4.6.2 \
 && CFLAGS="$CFLAGS_A0" CXXFLAGS="$CFLAGS_A0" FFLAGS="$FFLAGS_A0" FCFLAGS="$FFLAGS_A0" CPPFLAGS= LDFLAGS="$LDFLAGS_A0" \
    LIBnn=lib ./configure --prefix=/opt/R $CONFIGURE_A0 > /tmp/a0/configure.log \
 && make -j"$(nproc)" > /dev/null && make install > /dev/null \
 && grep -E '^(CC|CFLAGS|LDFLAGS) =' Makeconf | tee /opt/R/a0-flags.txt \
 && cd / && rm -rf /tmp/a0/R-4.6.2 \
 && /opt/R/bin/Rscript -e 'cat(R.version.string, "\n")'
