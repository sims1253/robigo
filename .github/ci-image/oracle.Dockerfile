# A GNU R oracle built from the pinned upstream tarball (docs/g3.md L3, L4):
# robigo-oracle:4.6.2, the GNU R that robigo-rt is gated on. 4.5.0 stays
# Debian's r-base-core in the CI image (Dockerfile, target base).
#
#   scripts/ci-image.sh build oracle-4.6.2      # build locally
#   scripts/build-oracle.sh --image 4.6.2       # same, tagged robigo-oracle:4.6.2
#
# Why a source build and not a Debian package: Debian ships 4.6.2 only in
# unstable, which moves (a superseded package leaves the archive) and has
# another glibc than trixie. GNU R uses the C library's sinpi & co., so a
# different glibc changes results in the last bit and a 4.5.0 -> 4.6.2
# comparison would mix R's changes with the C library's. This image is
# the 4.5.0 oracle's platform (Debian trixie, its gcc, glibc, reference
# BLAS/LAPACK, tzdata) with only R replaced: the tarball whose sha256
# scripts/r-tarballs.sha256 pins, configured as Debian's debian/rules
# configures r-base (layout under /usr, external BLAS/LAPACK, the same
# compiler flags and Renviron), without Debian's patches, which change
# only build reproducibility and the Renviron line reproduced below.
FROM debian:trixie
ARG DEBIAN_FRONTEND=noninteractive
ARG R_VERSION
LABEL org.opencontainers.image.source=https://github.com/sims1253/robigo
LABEL org.opencontainers.image.description="robigo oracle: Debian trixie with GNU R built from the pinned upstream tarball"
COPY oracle-apt.txt r-tarballs.sha256 /tmp/robigo-oracle/
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends $(sed -e 's/#.*//' /tmp/robigo-oracle/oracle-apt.txt) > /dev/null \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; test -n "$R_VERSION"; cd /tmp/robigo-oracle \
 && curl -fsSLO "https://cran.r-project.org/src/base/R-${R_VERSION%%.*}/R-${R_VERSION}.tar.gz" \
 && grep -F "  R-${R_VERSION}.tar.gz" r-tarballs.sha256 | sha256sum -c - \
 && tar -xzf "R-${R_VERSION}.tar.gz" && cd "R-${R_VERSION}" \
 && flags() { DEB_BUILD_MAINT_OPTIONS=optimize=-lto dpkg-buildflags --get "$1"; } \
 && R_SHELL=/bin/bash R_PAPERSIZE=letter R_BROWSER=xdg-open R_PDFVIEWER=/usr/bin/xdg-open \
    R_BZIPCMD=/bin/bzip2 R_GZIPCMD="/bin/gzip -n" R_UNZIPCMD=/usr/bin/unzip R_ZIPCMD=/usr/bin/zip \
    R_PRINTCMD=/usr/bin/lpr PAGER=/usr/bin/pager PERL=/usr/bin/perl TAR=/bin/tar LIBnn=lib \
    AWK=/usr/bin/awk SED=/bin/sed GETWD=/bin/pwd \
    CFLAGS="$(flags CXXFLAGS) $(flags CPPFLAGS)" CXXFLAGS="$(flags CXXFLAGS) $(flags CPPFLAGS)" \
    FFLAGS="$(flags FFLAGS)" FCFLAGS="$(flags FFLAGS)" CPPFLAGS= LDFLAGS="$(flags LDFLAGS)" \
    rsharedir=/usr/share/R/share rincludedir=/usr/share/R/include rdocdir=/usr/share/R/doc \
    ./configure --prefix=/usr --with-cairo --with-jpeglib --with-readline --with-tcltk \
      --mandir=/usr/share/man --infodir=/usr/share/info --datadir=/usr/share/R/share \
      --includedir=/usr/share/R/include --with-blas --with-lapack --enable-R-profiling \
      --enable-R-shlib --enable-memory-profiling --without-recommended-packages --disable-java > /dev/null \
 && make -j"$(nproc)" > /dev/null \
 && make install > /dev/null \
 # Debian's set_R_LIBS_USER_and_SITE_in_Renviron.patch, and the site libraries it names
 && sed -i "s|^R_LIBS_SITE=.*|R_LIBS_SITE=\${R_LIBS_SITE:-'/usr/local/lib/R/site-library:%S'}|" /usr/lib/R/etc/Renviron \
 && mkdir -p /usr/local/lib/R/site-library /usr/lib/R/site-library \
 && cd / && rm -rf /tmp/robigo-oracle \
 && test "$(Rscript -e 'cat(format(getRversion()))')" = "$R_VERSION" \
 && R --version | head -1
ENV R_HOME=/usr/lib/R
# .github/actions/setup skips its installs when this file exists.
RUN echo "oracle-$R_VERSION" > /etc/robigo-ci-image
