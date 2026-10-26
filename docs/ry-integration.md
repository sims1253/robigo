# robigo and ry

[ry](https://github.com/sims1253/ry) is a static type checker for R. robigo is a
runtime. They can help each other in both directions.

## Runtime types as an oracle for ry (available now)

robigo can record the types it actually observes, in the same "scope exit"
shape that `ry dump-facts` exports:

```sh
ry dump-facts script.R --format json > facts.json
ROBIGO_TYPE_PROFILE=observed.json robigo script.R
scripts/compare-ry-types.py facts.json observed.json
```

For each binding of the top level and of every function scope that ran,
the comparison reports:

- `agree` — ry's mode and length match every observed value;
- `CONFLICT` — ry claims something a real run contradicted: an unsound fact
  or a code path ry does not model;
- `opportunity` — ry says `opaque`/unknown, but every run saw one mode (for
  example accumulators like `s <- s + i` in a loop, or values derived from
  parameters). These are the facts a runtime would most like to have.

Scopes are matched by the position of the `function` keyword, as in ry's
facts. The profile records mode, length, class, attribute names and
NA-presence; ry's facts currently have no NA or attribute information (see
below), so those fields are reported but not compared yet.

Running this over a corpus of scripts and packages (robigo executes base R's
own R code, and will be able to run packages' R code) gives ry a stream of
counterexamples and missed precision, without hand-written expectations.

## Static types for the runtime (planned)

robigo's bytecode VM already specializes dynamically: arithmetic on
attribute-free scalars is unboxed behind cheap checks, and variable
updates happen in place. Static facts would let it do what dynamic checks
cannot:

- **Frame slots instead of environments.** If ry proves a function body
  never reflects on its frame (no `environment()`, `eval`, `assign`,
  `sys.function`, NSE helpers — the "blockers" ry already tracks for
  reference facts), locals can live in VM registers. That removes most of
  the remaining cost of closure calls.
- **Strictness.** Parameters ry can show are always forced, and whose
  arguments have no side effects, can be evaluated eagerly instead of
  through promises (OOPSLA'21 found 72.9% of parameters are strict).
- **Guard hoisting.** Facts about scalar/no-attribute parameters let the
  VM check types once at function entry instead of at every operation
  (Ř's "contextual dispatch").

What the runtime needs from the facts, beyond today's schema: NA-freeness
(`na: free|maybe`), attribute information (`attrs: none|names|dim|unknown`),
exact vs coercible mode (a "double" that is never an integer), and the
blocker information for frames, all keyed by source span. Facts would be
used as guarded hints, never trusted blindly, since R allows redefining
almost everything at run time.
