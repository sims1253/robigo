#!/bin/sh
# Build GNU R's base-package native code from the R 4.5.0 sources for
# robigo (docs/core.md §8): the packages' own src/ files, compiled with
# GNU R's Makeconf flags against GNU R's internal headers with
# -DTESTING_WRITE_BARRIER (Defn.h then calls the exported accessor
# functions instead of reading fields, so the objects run on robigo's
# libR). The results land in .robigo/natives/<pkg>/<pkg>.so, where
# scripts/robigo-home.sh installs them into robigo's R_HOME.
#
# Usage: scripts/build-natives.sh [PKG...]        (default: stats)
#   R_HOME   GNU R 4.5.0 (its etc/Makeconf gives compilers and flags)
#   RSRC     the R 4.5.0 sources (default ~/.cache/robigo-research/rsrc/
#            R-4.5.0; downloaded from CRAN and checked when missing)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
pkgs=${*:-stats}
gnu=${R_HOME:-$(R RHOME)}
RSRC=${RSRC:-$HOME/.cache/robigo-research/rsrc/R-4.5.0}
out=$root/.robigo/natives
build=$out/build
SHA256=3b33ea113e0d1ddc9793874d5949cec2c7386f66e4abfb1cef9aec22846c3ce1

if [ ! -f "$RSRC/src/include/Defn.h" ]; then
    mkdir -p "$(dirname "$RSRC")"
    tgz=$(dirname "$RSRC")/R-4.5.0.tar.gz
    [ -f "$tgz" ] || curl -sSL -o "$tgz" https://cran.r-project.org/src/base/R-4/R-4.5.0.tar.gz
    echo "$SHA256  $tgz" | sha256sum -c - > /dev/null
    tar xzf "$tgz" -C "$(dirname "$RSRC")"
fi

# GNU R's configured headers (config.h, Rconfig.h, ...) for Defn.h
if [ ! -f "$build/src/include/config.h" ]; then
    mkdir -p "$build"
    (cd "$build" && "$RSRC/configure" --with-x=no --without-recommended-packages \
        --disable-java --with-readline=no > configure.log 2>&1 &&
        make -C src/include > make.log 2>&1) || {
        echo "build-natives: configuring the R sources failed (see $build/configure.log)" >&2
        exit 1
    }
fi

# a variable of GNU R's Makeconf (what `R CMD config` prints)
share=$(sed -n 's/^R_SHARE_DIR=//p' "$gnu/bin/R" | head -1)
[ -d "$gnu/share" ] && share=$gnu/share
conf() {
    printf 'include $(R_HOME)/etc/Makeconf\nprint:\n\t@echo $(%s)\n' "$1" |
        make -s -f - R_HOME="$gnu" R_SHARE_DIR="$share" print
}
CC=$(conf CC)
CFLAGS="$(conf CFLAGS) $(conf CPICFLAGS)"
FC=$(conf FC)
FFLAGS="$(conf FFLAGS) $(conf FPICFLAGS)"
SAFE_FFLAGS="$(conf SAFE_FFLAGS) $(conf FPICFLAGS)"
LIBS="$(conf LAPACK_LIBS) $(conf BLAS_LIBS) $(conf FLIBS)"

# the file list of a package Makefile.in variable (continuation lines joined)
sources() {
    sed -e ':a' -e '/\\$/N; s/\\\n//; ta' "$1" | sed -n "s/^$2 *= *//p"
}

for pkg in $pkgs; do
    src=$RSRC/src/library/$pkg/src
    obj=$out/$pkg/obj
    rm -rf "$obj" && mkdir -p "$obj"
    for f in $(sources "$src/Makefile.in" SOURCES_C); do
        $CC $CFLAGS -DTESTING_WRITE_BARRIER -DHAVE_CONFIG_H -I"$build/src/include" \
            -I"$RSRC/src/include" -I"$RSRC/src/main" -I"$src" -c "$src/$f" -o "$obj/${f%.c}.o" 2>> "$obj/cc.log"
    done
    for f in $(sources "$src/Makefile.in" SOURCES_F); do
        # R compiles PORT (portsrc.f) with SAFE_FFLAGS (-ffloat-store)
        flags=$FFLAGS
        [ "$f" = portsrc.f ] && flags=$SAFE_FFLAGS
        $FC $flags -c "$src/$f" -o "$obj/${f%.f}.o" 2>> "$obj/cc.log"
    done
    # the marker robigo's dyn.load looks for before loading a base
    # package's shared object instead of serving it with Rust ports
    echo 'int robigo_base_native = 1;' > "$obj/robigo-marker.c"
    $CC $CFLAGS -c "$obj/robigo-marker.c" -o "$obj/robigo-marker.o"
    $CC -shared -o "$out/$pkg/$pkg.so" "$obj"/*.o $LIBS
    echo "build-natives: $out/$pkg/$pkg.so"
done
