# conformance-sweep-base: base primitive type and attribute rules

## 2026-10-31: bitwise type rules, all.names counting, length<- and row.names

- why: four `fastr` clusters failed on GNU R rules in C code that was
  either approximated or missing: bitwise operations (12 cases),
  `all.names`/`all.vars` (5), `length<-`/`length(x) <- v` (5) and
  `unclass()` of a data frame (7). All are single-function ports from the
  R sources, not evaluator changes.
- what:
  - `builtins/misc.rs`: `bitwAnd/Or/Xor/ShiftL/ShiftR/Not` now follow
    `relop.c:BIT` and `bitwiseShiftL/bitwiseShiftR`: only REAL operands
    are coerced to INTSXP (a shift count is coerced whenever it is not
    already integer), both operands must then have the same type, and
    anything but integer is `unimplemented type '<type>' in '<name>'`.
    Shift counts outside 0..31 give NA. Coercion uses the checked
    `coerce_vector` path (a pairlist or one-element list coerces; a
    longer list element errors "'list' object cannot be coerced to type
    'integer'"). Each operand emits its own `coerceVector` warnings in
    operand order, before the type check; list and pairlist elements go
    through `asInteger` one by one, so each element warns separately,
    only a nonempty atomic element converts (a nested list or expression
    element is silently NA), and a symbol warns "'symbol' object cannot
    be coerced to type 'integer'" and coerces to NULL (then fails the
    type check), as `coerceSymbol` does.
  - `builtins/parsing.rs`: `list.c:do_allnames` maps `max.names`
    `-1` to unlimited and every other negative value (including NA) to
    none, treats `unique = NA` as true, and `namewalk` drops the call
    head when `functions = FALSE` without walking it.
  - `builtins/bind.rs`: `length<-` uses `builtin.c:asVecSize`
    (already in `io.rs`) for the size argument, reports the replacement
    call for errors (`Flow::error`, not the builtin's own call), keeps
    an unchanged object (`xlengthgets` returns `x` when the length is
    the same, so its attributes survive), warns
    `length of NULL cannot be changed` and returns NULL, and says
    `cannot set length of non-(vector or list)`. Size errors come first
    (`asVecSize` runs before `xlengthgets`), so a symbol with
    `length(x) <- NA_real_` reports the size error, and the character
    size branch of `asVecSize` now applies the too-large bound too.
  - `print.rs`: attribute printing reads each attribute through
    `attrib.c:getAttrib`, which expands a compact `c(NA, n)` row.names,
    so `print(unclass(df))` and any list with compact row names print
    `1:n` as GNU R does. `attributes()` already used `get_attrib`.
- tests: `tests/r/basics/base-type-rules.R`, recorded with GNU R 4.5.0
  in `robigo-oracle:4.5.0`; it passes the interpreter, VM and JIT tiers
  and the full conformance suite stays green.
- result: `fastr`
  `TestBuiltin_(bitwise|allnames|lengthassign|unclass)` 114/143 ->
  143/143; 29 new ratchet markers. Full `fastr` 31,153/31,374 ->
  31,183/31,374 (+30, zero regressions; the 30th case,
  `copyDFattr/e50999e6a857194e`, is fixed by the row.names printing but
  left unmarked like its three already-passing siblings, which CI has not
  absorbed). `gnur-examples` base attribute examples: 5/6, no change.
- Note: `length_gets` now returns the object unchanged when the length is
  the same (as `xlengthgets` does); its only caller is `length<-`.
- Perf: the first version sent every bitwise call through `coerce_vector`,
  which allocates a `Value` per operand. `rbench/awf/Storage` (its RNG is
  `bitwAnd(seed * 1309 + 13849, 65535)`) went 559.2M -> 567.8M
  instructions locally (+1.54%) and failed CI's perf gate. The common
  integer/double path now converts with `to_int` directly and builds no
  `Value`; it is back to 560.7M (+0.27%), inside the 0.5% gate, while the
  list/pairlist/symbol paths keep `coerceVector`'s checks.
