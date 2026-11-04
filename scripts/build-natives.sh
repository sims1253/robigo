#!/bin/sh
# Build GNU R's base-package native code from the R 4.5.0 sources for
# robigo (docs/core.md §8): the packages' own src/ files, compiled with
# GNU R's Makeconf flags against GNU R's internal headers with
# -DTESTING_WRITE_BARRIER (Defn.h then calls the exported accessor
# functions instead of reading fields, so the objects run on robigo's
# libR). The results land in .robigo/natives/<pkg>/<pkg>.so, where
# scripts/robigo-home.sh installs them into robigo's R_HOME.
#
# Usage: scripts/build-natives.sh [PKG...]
#   (default: every base package with native code that robigo has no port
#   of; stats last, as the only one that needs static LAPACK and BLAS)
#   R_HOME   GNU R 4.5.0 (its etc/Makeconf gives compilers and flags)
#   RSRC     the R 4.5.0 sources (default ~/.cache/robigo-research/rsrc/
#            R-4.5.0; downloaded from CRAN and checked when missing)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
pkgs=${*:-tools parallel utils grDevices graphics grid splines stats}
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
# R 4.5's configure requires libcurl >= 7.28 headers with https and offers
# no opt-out. Machines without libcurl4-openssl-dev get a self-provisioned
# prefix: the distribution's libcurl dev+runtime .debs, downloaded (no
# root needed) and extracted next to the build tree, with curl-config on
# PATH for configure's feature checks. An already-working curl (curl-config
# finds headers) is left alone; CPPFLAGS/LDFLAGS from the environment are
# kept.
curl_prefix=
if [ ! -f "$build/src/include/config.h" ] && [ ! -x "$(command -v curl-config)" ]; then
    curl_prefix=$out/curl-prefix
    # curl-config inside the prefix marks a COMPLETE extraction: the .debs
    # unpack into prefix.tmp, published in one move, so an interrupted run
    # cannot leave a half prefix that later runs reuse.
    if [ ! -f "$curl_prefix/prefix/usr/bin/curl-config" ]; then
        echo "build-natives: no curl-config; fetching libcurl debs into $curl_prefix" >&2
        mkdir -p "$curl_prefix"
        if ! (cd "$curl_prefix" && { apt-get download libcurl4-openssl-dev libcurl4t64 ||
                apt-get download libcurl4-openssl-dev libcurl4 ; } >download.log 2>&1); then
            echo "build-natives: could not download libcurl (see $curl_prefix/download.log);" >&2
            echo "  install libcurl4-openssl-dev or set CPPFLAGS/LDFLAGS to a curl prefix" >&2
            exit 1
        fi
        # Every extraction must succeed: this runs inside a condition, where
        # set -e is off and a later .deb could hide an earlier failure.
        if ! (cd "$curl_prefix" && rm -rf prefix.tmp && mkdir prefix.tmp &&
                for d in *.deb; do dpkg-deb -x "$d" prefix.tmp/ || exit 1; done &&
                mv prefix.tmp prefix); then
            echo "build-natives: could not extract the libcurl debs" >&2
            rm -rf "$curl_prefix/prefix.tmp"
            exit 1
        fi
    fi
    p=$curl_prefix/prefix
    PATH="$p/usr/bin:$PATH"
    export PATH
    CPPFLAGS="${CPPFLAGS:-} -I$p/usr/include/$(gcc -dumpmachine 2>/dev/null || echo x86_64-linux-gnu)"
    LDFLAGS="${LDFLAGS:-} -L$p/usr/lib/$(gcc -dumpmachine 2>/dev/null || echo x86_64-linux-gnu)"
    export CPPFLAGS LDFLAGS
fi
# A tree configured before cairo's headers were installed has no cairo
# devices (HAVE_WORKING_CAIRO): configure it again once they are there.
if [ -f "$build/src/include/config.h" ] && ! grep -q '^#define HAVE_WORKING_CAIRO' "$build/src/include/config.h" &&
    pkg-config --exists cairo 2> /dev/null; then
    rm -f "$build/src/include/config.h"
fi
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
# Static LAPACK/BLAS so loading the .so does not map liblapack.so.3 and
# libblas.so.3 at startup (their relocations cost ~2M instructions per
# run); only the objects the package references are pulled in.
# --as-needed drops libgfortran, which these objects never reference,
# and anything else the package does not use.
# -Bsymbolic-functions binds the package's calls to its own global
# functions at link time (they are internal; nothing may interpose
# them), so the loader does not resolve ~700 self-references per run.
LINKFLAGS="-Wl,--as-needed -Wl,-Bsymbolic-functions"
LINALG="-Wl,-Bstatic $(conf LAPACK_LIBS) $(conf BLAS_LIBS) -Wl,-Bdynamic"
FLIBS=$(conf FLIBS)

# the file list of a package Makefile.in variable (continuation lines joined)
sources() {
    # configure turns @BUILD_AQUA_TRUE@ (macOS only) into `#`, commenting
    # out the rest of the line
    sed -e ':a' -e '/\\$/N; s/\\\n//; ta' "$1" | sed -n "s/^$2 *= *//p" |
        sed -e 's/@BUILD_AQUA_TRUE@/#/' -e 's/#.*//'
}

# a substitution configure made (config.status), e.g. CAIRO_CPPFLAGS
subst() {
    sed -n "s/^S\[\"$1\"\]=\"\(.*\)\"$/\1/p" "$build/config.status"
}

# grDevices' cairo devices live in a module of their own, cairo.so next to
# grDevices.so, which R_cairoCdynload loads on the first cairo device
# (src/library/grDevices/src/cairo/Makefile.in: cairoBM.c and X11's
# rbitmap.o, with configure's CAIRO_CPPFLAGS, CAIRO_LIBS and BITMAP_LIBS).
# Built only where configure found a working cairo, as in GNU R.
cairo_module() {
    grep -q '^#define HAVE_WORKING_CAIRO' "$build/src/include/config.h" || {
        echo "build-natives: no cairo (install libcairo2-dev and libpango1.0-dev): no cairo devices" >&2
        return 0
    }
    csrc=$RSRC/src/library/grDevices/src/cairo
    cobj=$out/grDevices/cairo-obj
    rm -rf "$cobj" && mkdir -p "$cobj"
    for f in "$csrc/cairoBM.c" "$RSRC/src/modules/X11/rbitmap.c"; do
        $CC $CFLAGS -DTESTING_WRITE_BARRIER -DHAVE_CONFIG_H -I"$build/src/include" \
            -I"$RSRC/src/include" -I"$RSRC/src/main" $(subst CAIRO_CPPFLAGS) \
            -I"$RSRC/src/modules/X11" -I"$csrc" -c "$f" -o "$cobj/$(basename "${f%.c}").o" 2>> "$cobj/cc.log"
    done
    $CC -shared -o "$out/grDevices/cairo.so" "$cobj"/*.o $(subst BITMAP_LIBS) $(subst CAIRO_LIBS) $LINKFLAGS
    echo "build-natives: $out/grDevices/cairo.so"
}

for pkg in $pkgs; do
    src=$RSRC/src/library/$pkg/src
    obj=$out/$pkg/obj
    rm -rf "$obj" && mkdir -p "$obj"
    for f in $(sources "$src/Makefile.in" SOURCES_C); do
        in=$src/$f
        # utils' size.c asks for direct field access (USE_RINTERNALS), GNU
        # R's object layout; compiled without it, it uses the accessor
        # functions like every other file, and natives-layout.h gives it
        # the layout's sizes it reports
        if grep -q '^#define USE_RINTERNALS' "$in"; then
            sed -e 's|^#define USE_RINTERNALS|/* robigo: accessor functions */|' \
                -e "s|^#include <Defn.h>|&\\n#include \"$root/scripts/natives-layout.h\"|" \
                "$in" > "$obj/$f"
            in=$obj/$f
        fi
        $CC $CFLAGS -DTESTING_WRITE_BARRIER -DHAVE_CONFIG_H -I"$build/src/include" \
            -I"$RSRC/src/include" -I"$RSRC/src/main" -I"$src" -c "$in" -o "$obj/${f%.c}.o" 2>> "$obj/cc.log"
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
    # the package's own libraries (grDevices: -lz); AQUA_LIBS is macOS's.
    # Words still holding unexpanded make/autoconf variables ($(BLAS_LIBS),
    # @R_OPENMP_CFLAGS@) are dropped: stats' static LAPACK/BLAS objects
    # come via $LINALG below instead.
    pkglibs=$(sources "$src/Makefile.in" PKG_LIBS | sed 's/\$(AQUA_LIBS)//' | tr -s ' \n' '\n' | grep -vE '[$@]' | tr '\n' ' ')
    # R's libR provides xerbla (src/main/print.c: BLAS/LAPACK errors
    # become R errors); the static LAPACK objects would otherwise pull
    # in LAPACK's own xerbla and with it libgfortran's I/O. A local copy
    # of R's keeps the .so self-contained and behaviour as in GNU R.
    cat > "$obj/xerbla.c" <<'EOF'
#include <string.h>
void Rf_error(const char *, ...);
void xerbla_(const char *srname, int *info, size_t srname_len)
{
    char buf[21];
    int len = (srname_len > 20) ? (int)srname_len : 20;
    strncpy(buf, srname, len);
    buf[len] = '\0';
    Rf_error("BLAS/LAPACK routine '%6s' gave error code %d", buf, -(*info));
}
EOF
    $CC $CFLAGS -c "$obj/xerbla.c" -o "$obj/xerbla.o" 2>> "$obj/cc.log"
    # (Only what asks for it: static LAPACK is not on every machine.)
    linalg=
    if grep -qE 'BLAS_LIBS|LAPACK_LIBS' "$src/Makefile.in"; then linalg=$LINALG; fi
    $CC -shared -o "$out/$pkg/$pkg.so" "$obj"/*.o $pkglibs $LINKFLAGS $linalg $FLIBS
    echo "build-natives: $out/$pkg/$pkg.so"
    if [ "$pkg" = grDevices ]; then cairo_module; fi
done
