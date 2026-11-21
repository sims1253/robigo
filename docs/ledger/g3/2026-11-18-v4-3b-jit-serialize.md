## 2026-11-18: V4 step 3b: a closure robigo-rt's JIT compiled is serialized as byte code

When GNU R's JIT compiles a closure (`R_CheckJIT`, then `R_cmpfun`,
eval.c), it installs `R_cmpfun1`'s BCODESXP as the closure's BODY, and
serialize's `WriteItem` writes that body as it is. robigo-rt's JIT keeps
its own code beside the source body. So the writer refused such a closure
("serializing a byte-compiled closure is not implemented yet"). That
refusal stopped L's `mklazycomp` step: makebasedb.R runs with the JIT on,
and the closures it has called by the time it writes them are compiled.

### What landed

- `serialize/write.rs`, in `WriteItem`'s walk, after the
  `_R_COMPILE_PKGS_` compile on write: a closure robigo-rt's JIT has
  compiled (its own code, its body not a BCODESXP) is written as
  `R_cmpfun1(s)`'s result. That is the closure GNU R's JIT would have
  installed. Where the compiler declines (the result has no byte code), the
  closure is marked NOJIT, as `R_cmpfun` marks it, and written as its
  source.
- `R_disable_bytecode` does not stop the write. In GNU R it stops byte
  code running, not an installed body being written. With
  `R_DISABLE_BYTECODE=1`, a base closure that carries the byte-code mark
  is still written as byte code. The setting still gates the compile on
  write, as serialize.c:1040 does.
- `compile_on_write` is an associated function, `Self::compile_on_write`
  (#856's OCR low).
- serialize/mod.rs: "not yet" now names only a promise made by robigo-rt's
  compiled code.

### The fixtures

- `tests/rt/jit/serialize-write.R`, recorded on GNU R 4.6.2, covers a
  closure in each JIT state, in versions 2 and 3:
  - a loop, compiled at its first call;
  - a small closure, MAYBEJIT (flags 0x0404) after one call and compiled
    after two;
  - one never called;
  - a local closure marked NOJIT (0x0204);
  - a compiled closure unserialized and run.

  It needs the compiler namespace to load (#860, L's `.AutoloadEnv`), so
  its marker lands with that PR. On a local merge with #860, robigo-rt's
  output is the same as GNU R's, byte for byte.
- `tests/rt/jit/serialize-write-shadow.R`, recorded, not ratcheted: the
  first residual difference below.

### The two residual differences

1. **An environment changed between the JIT moment and the write.** GNU R
   writes the code its JIT made at the closure's call. robigo-rt compiles
   at the write. The compiler's decisions depend on the environment, so
   the two differ when a binding they read changed in between. The case:
   a closure calls `length`, and `length` is shadowed in its environment
   before the write. GNU R writes the inlined `length` behind a base guard
   (757 bytes, the same before and after). robigo-rt writes a generic call
   (837 bytes after). This cannot occur in base's build: makebasedb.R does
   not rebind base's environment between a JIT moment and its write. In a
   package build it occurs only when the package's own top-level code
   calls one of its closures and then shadows a base function that the
   closure uses. Removing it means installing the JIT's byte code at the
   JIT moment (V4 step 3's follow-up: robigo-rt keeps `R_cmpfun1`'s result
   with its own code).
2. **Promises made by compiled code keep the refusal.** robigo-rt's
   compiled code makes promises whose code is its own IR, not a BCODESXP,
   so they still raise "serializing a promise of byte code is not
   implemented yet". GNU R writes such a promise with its byte-code PRCODE.
   The build can reach one. `makeLazyLoadDB` and makebasedb.R write the
   bindings unforced (`getVarsFromFrame(.., FALSE)`), so an unforced
   argument in a closure's environment is written as a promise. An example
   is a closure that a compiled factory returned at the package's top
   level. The build then stops loudly at that write; it does not write a
   wrong value. It did not occur in base's or tools' mklazycomp on the
   merge below. robigo-rt's base bindings are promises of
   `..robigo_base_fetch`, but those come from the loader, not from compiled
   code. Removing it means a promise that keeps GNU R's byte-code PRCODE
   (the same follow-up as 1).

### mklazycomp

On a throwaway merge (this branch, #860, main and L's set: #884's
`scripts/rt-base-build.sh` with its shim, #823 #836 #872 #880 #852 #875
#877 #854), L's build runs `base-mklazycomp` and `tools-mklazycomp` to the
end: 5 of 10 steps, where L reached 4.

- base.rdb: 1152 of 1152 functions are byte code, as in the oracle's.
  `%in%`'s byte code is identical. Only 27 of 1179 records are identical,
  because robigo-rt's base bindings are written as PROMSXP of
  `..robigo_base_fetch` (`getVarsFromFrame(force = FALSE)`). That is the
  base loader's difference (L, INTERIM(L5)).
- The remaining stops are owned by others:

  | step | stop | owner |
  |---|---|---|
  | tools-all | `readChar` | L |
  | compiler-all | `lengths()` of an object | E/S1 |
  | base-mkdesc | `Sys.getlocale` | L |
  | vdesc, vns | `dir.create` | B |

### Review

Sol (codex `gpt-6.1-sol`, high), round 1: APPROVE, one nit. With
`R_DISABLE_BYTECODE=1`, marked base closures were written as source, but
GNU R writes their BCODESXP bodies. Fixed: the setting no longer gates the
write of a JIT-compiled closure.

### Verification

- The full `scripts/rt-ci.py` exits 0. Conformance is 574/583; the 9
  failing cases are unratcheted (the 5 `bytecode/*` cases,
  `library-dynam-tools`, `compile-pkgs`, `jit-write`, `jit-write-shadow`;
  all need the compiler to load). fail-alloc shows 0 differences in 2800
  runs.
- After Sol's nit: the `tests/rt/serialize` slice and the cli tests pass.
- Callgrind quick check against main `deb9c8d32`'s binary, in millions of
  instructions:

  | row | main | 3b |
  |---|---:|---:|
  | plain | 1894.3 | 1893.7 |
  | named | 1840.8 | 1840.9 |
  | superassign | 4908.2 | 4899.1 |
  | guard-subassign-dbl | 5108.4 | 5108.9 |

  vm.rs is not touched.
- Lint is clean.
