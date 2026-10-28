#!/bin/sh
# Build a GNU R base package's native code from the R 4.5.0 sources for
# robigo (docs/core.md §8): GNU R's Defn.h defines USE_RINTERNALS unless
# TESTING_WRITE_BARRIER is set, so compiling with -DTESTING_WRITE_BARRIER
# turns every internal accessor into a call of an exported function, which
# robigo's executable provides. Runs in a build image (robigo-oracle:4.5.0
# + gfortran + r-base-dev) against a configured R source tree.
#
#   build-base-native.sh PKG OUTDIR
#     RSRC   R 4.5.0 sources (default ~/.cache/robigo-research/rsrc/R-4.5.0)
#     RBUILD a configure'd build directory for RSRC (src/include/config.h)
set -e
pkg=$1
out=$2
RSRC=${RSRC:-$HOME/.cache/robigo-research/rsrc/R-4.5.0}
RBUILD=${RBUILD:-/tmp/rbuild}
mkdir -p "$out"
if [ ! -f "$RBUILD/src/include/config.h" ]; then
  mkdir -p "$RBUILD"
  docker run --rm -u "$(id -u):$(id -g)" -v "$RSRC:/rsrc:ro" -v "$RBUILD:$RBUILD" -w "$RBUILD" robigo-build:4.5.0 \
    sh -c '/rsrc/configure --with-x=no --without-recommended-packages --disable-java --with-readline=no > configure.log 2>&1 && make -C src/include > make.log 2>&1'
fi
docker run --rm -u "$(id -u):$(id -g)" -v "$RSRC:/rsrc:ro" -v "$RBUILD:$RBUILD:ro" -v "$out:$out" -w "$out" robigo-build:4.5.0 sh -c "
  S=/rsrc/src/library/$pkg/src
  for f in \$S/*.c; do
    gcc -std=gnu2x -DTESTING_WRITE_BARRIER -DHAVE_CONFIG_H -I/rsrc/src/include -I$RBUILD/src/include -I/rsrc/src/main -I\$S -fPIC -O2 \
      -c \$f -o \$(basename \$f .c).o 2>>cc.log || echo \"skipped \$f (included by another file)\" >> cc.log
  done
  for f in \$S/*.f; do [ -f \$f ] || continue; gfortran -fPIC -O2 -c \$f -o \$(basename \$f .f).o; done 2>>cc.log
  # the marker robigo's dyn.load looks for before loading a base package's
  # shared object instead of serving it with Rust ports
  echo 'int robigo_base_native = 1;' > robigo-marker.c && gcc -fPIC -c robigo-marker.c
  gcc -shared -o $pkg.so *.o -lgfortran -lm -lblas -llapack
"
echo "$out/$pkg.so"
