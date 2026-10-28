#!/bin/sh
# Build robigo's R_HOME (docs/core.md §8): the tree GNU R's package build
# path expects, with robigo as its R. `R CMD INSTALL` and
# `install.packages()` then run GNU R's own scripts and R code (bin/R,
# bin/Rcmd, bin/INSTALL, tools:::.install_packages) with robigo as
# bin/exec/R, compile against robigo's headers, and link with a libR.so
# that only carries the soname (the API comes from the robigo executable).
#
# Usage: scripts/robigo-home.sh [GNU_R_HOME [ROBIGO_BINARY [DEST]]]
#   GNU_R_HOME     an R 4.5.0 installation (default: $R_HOME): its bin
#                  scripts, etc/, share/ and base package library
#   ROBIGO_BINARY  default .robigo/bin/robigo
#   DEST           default .robigo/home (robigo finds it there when
#                  R_HOME is unset)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
gnu=${1:-${R_HOME:-$(R RHOME 2>/dev/null)}}
[ -n "$gnu" ] || { echo "robigo-home: set R_HOME or pass GNU R's R_HOME" >&2; exit 1; }
robigo=${2:-$root/.robigo/bin/robigo}
dest=${3:-$root/.robigo/home}
gnu=$(cd "$gnu" && pwd)
robigo=$(cd "$(dirname "$robigo")" && pwd)/$(basename "$robigo")
[ -x "$robigo" ] || { echo "robigo-home: no robigo binary at $robigo (scripts/build.sh)" >&2; exit 1; }
[ -f "$gnu/etc/Makeconf" ] || { echo "robigo-home: $gnu is not an R installation" >&2; exit 1; }

# GNU R's own locations of share/ and doc/ (Debian puts them in /usr/share/R)
dir_of() {
    v=$(sed -n "s/^$1=//p" "$gnu/bin/R" | head -1)
    for d in "$gnu/$2" "$v"; do
        if [ -n "$d" ] && [ -d "$d" ]; then echo "$d"; return; fi
    done
}
share=$(dir_of R_SHARE_DIR share)
doc=$(dir_of R_DOC_DIR doc)
[ -n "$share" ] && [ -f "$share/make/shlib.mk" ] || {
    echo "robigo-home: GNU R's share/ (make/shlib.mk) not found next to $gnu or at its R_SHARE_DIR" >&2
    exit 1
}

rm -rf "$dest"
mkdir -p "$dest/bin/exec" "$dest/etc" "$dest/lib" "$dest/library" "$dest/site-library"
dest=$(cd "$dest" && pwd)

# bin: GNU R's front-end scripts, pointed at this home; robigo is the R
# binary they exec. Rscript is a compiled program in GNU R; robigo takes
# Rscript's command line itself.
for f in "$gnu"/bin/*; do
    [ -f "$f" ] && cp "$f" "$dest/bin/"
done
sed -i \
    -e "s|^R_HOME_DIR=.*|R_HOME_DIR=$dest|" \
    -e "s|^R_SHARE_DIR=.*|R_SHARE_DIR=$dest/share|" \
    -e "s|^R_INCLUDE_DIR=.*|R_INCLUDE_DIR=$dest/include|" \
    -e "s|^R_DOC_DIR=.*|R_DOC_DIR=$dest/doc|" \
    "$dest/bin/R"
ln -s "$robigo" "$dest/bin/exec/R"
cat > "$dest/bin/Rscript" <<EOS
#!/bin/sh
R_HOME=$dest
export R_HOME
exec "$dest/bin/exec/R" "\$@"
EOS
chmod +x "$dest/bin/Rscript"

# etc: GNU R's, except that the site library is this home's own (GNU R's
# Renviron.site may name another R's library)
cp "$gnu"/etc/* "$dest/etc/"
cat > "$dest/etc/Renviron.site" <<'EOS'
## robigo: only this home's site library
R_LIBS_SITE="%S"
EOS

# include: robigo's headers; share, doc: GNU R's
ln -s "$root/crates/robigo-capi/include" "$dest/include"
ln -s "$share" "$dest/share"
[ -n "$doc" ] && ln -s "$doc" "$dest/doc"
[ -d "$gnu/modules" ] && ln -s "$gnu/modules" "$dest/modules"

# lib/libR.so: `-lR` at link time; the soname is all it carries
printf 'int robigo_libR_stub;\n' > "$dest/lib/libR-stub.c"
${CC:-cc} -shared -fPIC -Wl,-soname,libR.so -o "$dest/lib/libR.so" "$dest/lib/libR-stub.c"
rm "$dest/lib/libR-stub.c"

# library: GNU R's base packages (R code and, until robigo builds them,
# their shared objects); packages installed here are robigo's. A base
# package whose native code was built for robigo
# ($ROBIGO_NATIVES/<pkg>/<pkg>.so, default .robigo/natives, by
# scripts/build-natives.sh) gets that shared object in its libs/.
natives=${ROBIGO_NATIVES-$root/.robigo/natives}
for p in "$gnu"/library/*; do
    pkg=$(basename "$p")
    so=$natives/$pkg/$pkg.so
    if [ -n "$natives" ] && [ -f "$so" ]; then
        mkdir -p "$dest/library/$pkg/libs"
        for f in "$p"/*; do
            [ "$(basename "$f")" = libs ] || ln -s "$f" "$dest/library/$pkg/"
        done
        cp "$so" "$dest/library/$pkg/libs/"
        echo "robigo home: $pkg's native code built for robigo"
    else
        ln -s "$p" "$dest/library/$pkg"
    fi
done
echo "robigo home: $dest"
