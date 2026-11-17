#!/usr/bin/env bash
# A0: GNU R 4.6.2 from the pinned tarball, as a 2x2 of toolchain (the
# trixie image's, m0pad's) x flags (Debian's hardening, plain -g -O2), with
# one configure option set held fixed; plus a stock build (GNU R's
# defaults, image toolchain) and the local build's own recipe (host).
set -uo pipefail
cd ~/robigo-a0
DEB_C="-g -O2 -ffile-prefix-map=/=. -fstack-protector-strong -fstack-clash-protection -Wformat -Werror=format-security -fcf-protection -Wdate-time -D_FORTIFY_SOURCE=2"
DEB_F="-g -O2 -ffile-prefix-map=/=. -fstack-protector-strong -fstack-clash-protection -fcf-protection"
DEB_L="-Wl,-z,relro"
FIXED="--enable-R-shlib --enable-memory-profiling --without-recommended-packages --disable-java --with-x=no --without-tcltk --with-readline=no --without-cairo --without-libtiff --without-jpeglib --without-libpng"
LOCAL="--enable-R-shlib --without-recommended-packages --with-x=no --disable-java --without-tcltk --with-readline=no --without-cairo --without-libtiff --without-jpeglib --without-libpng"
STOCK="--without-recommended-packages --disable-java --with-x=no"
img() {  # name cflags fflags ldflags configure
  [ -n "$(docker images -q robigo-a0:$1)" ] && { echo "have image $1"; return; }
  s=$(date +%s)
  docker build -q -t robigo-a0:$1 --build-arg CFLAGS_A0="$2" --build-arg FFLAGS_A0="$3" --build-arg LDFLAGS_A0="$4" \
    --build-arg CONFIGURE_A0="$5" -f Dockerfile . > build-$1.log 2>&1 || { echo "!! image $1"; tail -20 build-$1.log; return; }
  echo "image $1 in $(( $(date +%s) - s )) s"
}
host() {  # name cflags fflags ldflags configure
  [ -x "host-$1/lib/R/bin/exec/R" ] && { echo "have host $1"; return; }
  s=$(date +%s)
  rm -rf src-$1 && mkdir src-$1 && tar -xzf R-4.6.2.tar.gz -C src-$1 --strip-components=1
  (cd src-$1 && CFLAGS="$2" CXXFLAGS="$2" FFLAGS="$3" FCFLAGS="$3" CPPFLAGS= LDFLAGS="$4" LIBnn=lib \
     ./configure --prefix="$HOME/robigo-a0/host-$1" $5 > ../configure-$1.log 2>&1 &&
   make -j"$(nproc)" > ../make-$1.log 2>&1 && make install > /dev/null 2>&1) || { echo "!! host $1"; tail -20 configure-$1.log make-$1.log 2>/dev/null | tail -20; return; }
  grep -E '^(CC|CFLAGS|LDFLAGS) =' src-$1/Makeconf > host-$1/a0-flags.txt
  rm -rf src-$1
  echo "host $1 in $(( $(date +%s) - s )) s"
}
img deb "$DEB_C" "$DEB_F" "$DEB_L" "$FIXED"
img plain "-g -O2" "-g -O2" "" "$FIXED"
img stock "-g -O2" "-g -O2" "" "$STOCK"
host deb "$DEB_C" "$DEB_F" "$DEB_L" "$FIXED"
host plain "-g -O2" "-g -O2" "" "$FIXED"
host local "-g -O2" "-g -O2" "" "$LOCAL"
echo done > build.done
