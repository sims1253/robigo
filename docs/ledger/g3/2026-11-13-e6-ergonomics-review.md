## 2026-11-13: E6 ergonomics review of the builtin-author API (Sol)

Reviewer: GPT-6.1 Sol (high), council member, did not write the code; read-only review of PR #648 at
c30ade12 (R1a #642 + N1a #645 + B1). Muse was unavailable (access error) and Gemini out of quota,
so Sol took the seat. Verdict: **pass with changes**.

Changes required before broad porting, ranked:
1. `build` with a structured outcome: result, plain-data metadata, and a terminal diagnostic
   description; the read interval ends before the diagnostic is raised; warning metadata returned
   from the successful attempt instead of captured flags (removes paste's too_long + nil sentinel,
   arith's flag resets, nchar's staging Vec). Warning emission stays ordered outside the closure.
2. Rooted-result helpers: a scope helper packaging out → root → invoke → return root
   (`s.capture(cx, |cx, out| cx.force(p, out))?`), and atomic root replacement
   (`current.replace_from(cx, next)`).
3. Detect a successful call that left `out` unwritten: `#[must_use]` on Out plus a checked
   invocation boundary distinguishing unwritten from an explicit nil.
4. One declaration per operator code in the registry (derive specialization and metadata together);
   validate eval digits; document how Visibility::Custom builtins set visibility.
5. An explicit standard-vector admission path after the ALTREP branch (no `Ref::len() == None`
   treated as 0), and a short author guide: scalar result, warning-producing vector loop, callback
   loop with bounded root scopes, error vs error_call(None) vs error_call(Some(call)).

Kept: unsized `&mut Cx` (acceptable for authors; document `?Sized` for generic adapters).
Still compiles (by design or pending): wrong diagnostic call choice, wrong registry metadata,
nonrepeatable host effects inside build's FnMut (convention), cross-isolate misuse (runtime check).
