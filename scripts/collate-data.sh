#!/usr/bin/env bash
# Regenerates robigo-collate-data's baked ICU4X data (crates/robigo-collate-data/data):
# ICU's root collator as GNU R 4.6.2 opens it in the oracle image, whose libicu
# is Debian trixie's 76.1 (CLDR 46, Unicode 16.0).
#
#   scripts/collate-data.sh
#
# The data is ICU4X datagen's (the version robigo-collate-data's icu_* crates
# are pinned to) over pinned, hash-checked sources:
# - the collation markers from ICU 76.1's export (icuexportdata_release-76-1),
#   CLDR 46.0.0 for datagen's locale data;
# - the NFD markers (the collator decomposes its input with them) from ICU
#   77.1's export, also Unicode 16.0: 76.1's lacks a field this datagen needs
#   (`cap`). Their decompositions and combining classes are checked against
#   Python's unicodedata 16.0.0 by robigo-collate-data's `nfd_is_unicode_16`.
# Only the root collation's markers are generated: no tailoring, no reordering.
# CARGO_TARGET_DIR and ROBIGO_COLLATE_WORK move the datagen build and the downloads.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=$root/crates/robigo-collate-data/data
datagen_version=2.3.0
work=${ROBIGO_COLLATE_WORK:-$HOME/.cache/robigo-research/collate-data}
mkdir -p "$work/src"

fetch() { # url sha256 file
  local f=$work/src/$3
  if ! echo "$2  $f" | sha256sum -c --status 2>/dev/null; then
    curl -fsSL -o "$f.part" "$1"
    echo "$2  $f.part" | sha256sum -c --quiet
    mv "$f.part" "$f"
  fi
  rm -rf "$work/src/${3%.zip}"
  mkdir "$work/src/${3%.zip}"
  unzip -q "$f" -d "$work/src/${3%.zip}"
}
fetch https://github.com/unicode-org/icu/releases/download/release-76-1/icuexportdata_release-76-1.zip \
  3d966342251393c53141fa6a5537ef7024b1f2ff1fe18a3f1c6e51aced67a11f icuexport-76-1.zip
fetch https://github.com/unicode-org/icu/releases/download/release-77-1/icuexportdata_release-77-1.zip \
  c5f0fadd1f3f2ed73d05b200821b2b4998725bcd026f1f58d200378327fdfb21 icuexport-77-1.zip
fetch https://github.com/unicode-org/cldr-json/releases/download/46.0.0/cldr-46.0.0-json-full.zip \
  de722ed2dd28a47de8358b9661fdcf20971e233c1ca1ca3b377db88875996567 cldr-46.0.0.zip

datagen=$work/datagen-$datagen_version/bin/icu4x-datagen
[ -x "$datagen" ] || cargo install icu4x-datagen --version "$datagen_version" --locked --root "$work/datagen-$datagen_version"

gen() { # icuexport dir markers...
  local src=$1 dir=$2
  shift 2
  # Han in radical-stroke order, as ICU's root has it (datagen's default,
  # implicit, is the DUCET's code point order)
  "$datagen" --format baked --cldr-root "$work/src/cldr-46.0.0" --icuexport-root "$work/src/$src" \
    --collation-root-han unihan \
    --locales und --use-separate-crates --no-internal-fallback --deduplication none \
    --overwrite -o "$work/$dir" --markers "$@"
}
gen icuexport-76-1 collation CollationRootV1 CollationJamoV1 CollationSpecialPrimariesV1 \
  CollationMetadataV1 CollationDiacriticsV1
gen icuexport-77-1 nfd NormalizerNfdDataV1 NormalizerNfdTablesV1

rm -rf "$out"
mkdir -p "$out"
cp "$work"/collation/*.rs.data "$work"/nfd/*.rs.data "$out"/
ls -l "$out"
