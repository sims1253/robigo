# C code's eval() of a promise (S1, item B)

GNU R's `eval(x, rho)` on a PROMSXP does more than `forcePromise(x)`. It sets
`R_Visible`, counts the evaluation, takes one eval-depth step (with
`R_CheckStack`) and runs the force with byte code inactive (`R_BCIntActive =
0`, saved and restored). So a promise that C code `eval()`s runs one level
deeper than a bare force, and an overflow there is not byte code's. robigo-rt
had ported that for `findFun3` only. This PR makes every ported site take it.

## The rule, in one place

- `eval::eval_in` (GNU R's `eval`): its symbol, promise and BCODESXP branches
  clear `bc_active` for C's evaluation (profile not `Bc`), as `eval` clears
  `R_BCIntActive`. Byte code's own promise code keeps its flag.
- `eval::eval_promise` (the `eval_promise` hook for rtbuiltins): `eval`'s
  `case PROMSXP` alone. The sites that hold a promise call it, as GNU R's call
  `eval(x, rho)`.
- `eval::force_arg_eval`: the same for a promise GNU R's binding would hold in
  an argument slot.

## The sites (GNU R 4.6.2, src/main unless named)

| GNU R site | robigo-rt | status |
|---|---|---|
| envir.c:findVar1mode:1511 | rtbuiltins envir.rs find_var1_mode | **ported here** (was a bare force) |
| envir.c:do_get:2493 (GET_VALUE; get, get0, exists) | rtbuiltins envir.rs get_value | **ported here** |
| envir.c:gfind:2536 (mget) | rtbuiltins envir.rs gfind | **ported here** |
| envir.c:FrameValues:3118 (as.list.environment, eapply) | rtbuiltins envir.rs do_env2list, do_eapply | **ported here** |
| envir.c:BuiltinValues:3238 (the same on base) | the same functions | **ported here** |
| subset.c:do_subset2_dflt:1011, R_subset3_dflt:1404 (`[[`, `$` on an environment) | rtbuiltins subset.rs env_value_sym | **ported here** |
| objects.c:GetObject:84 (UseMethod's object) | dispatch.rs get_object, Slot path | **ported here** (only an unevaluated promise, as GetObject tests) |
| eval.c:R_forceAndCall:2450 (lapply, vapply, mapply, forceAndCall) | apply.rs force_and_call | eval hook → now eval_promise |
| eval.c:evalList:3640, evalListKeepMissing:3730, DispatchOrEval:4144, bcEval DODOTS:8026 (`...` elements) | call.rs evallist_for, specials.rs eval_list_keep_missing / eval_args_hook, bcassign.rs dots_values | eval hook on the element: already the depth step; flag now cleared in eval_in |
| builtin.c:do_args:222/234 | builtin.rs do_args | eval hook (already) |
| deparse.c:deparse2buff:1428 | deparse.rs (PROMSXP case :160 eval hook; sym_prim reads base's value raw, as SYMVALUE) | already |
| envir.c:findVar1:1471 | rtbuiltins rng.rs rng_init | eval hook (already) |
| envir.c:R_DotsElt:1651 (...elt) | rtbuiltins envir.rs dots_elt_value | eval hook (already) |
| envir.c:findFun3:1915 | rtcore env/mod.rs find_fun3 | force_arg_eval / eval hook (already); flag now cleared |
| envir.c:getVarValInFrame:4338 | ns.rs get_var_val_in_frame | eval hook (already) |
| eval.c:do_eval:3985 | eval.rs do_eval | eval hook (already) |
| objects.c:findFunInEnvRange:178, findFunWithBaseEnvAfterGlobalEnv:202 | dispatch.rs fun_in_frame | eval hook (already) |
| objects.c:R_LookupMethod:270/278 | dispatch.rs lookup_from_top | eval hook (already; an evaluated promise read raw) |
| objects.c:getPrimitive:681, do_nextmethod:742/745/899 | dispatch.rs do_nextmethod | eval hook (already) |
| serialize.c:R_getVarsFromFrame:3260, do_lazyLoadDBfetch:3338 | serialize/mod.rs | eval hook (already) |
| subset.c:fixSubset3Args:1218 | specials.rs subset3_name | eval hook (already) |
| eval.c:SymbolValue:6895 (DO_BASEGUARD) | vm.rs base_guard via eval::symbol_value | **INTERIM(S1) → X**: one level short; vm.rs untouched here; case tests/rt/jit/base-guard-symbol-value.R |
| envir.c:R_getVarEx:2627 | none (robigo-capi's goes to robigo-runtime) | no port yet (C API) |
| envir.c:BuiltinValues:3226 (intern) | none | dead in GNU R (no caller passes intern) |
| objects.c:isBasicClass:315, print.c:PrintObjectS4:347 | none | no port yet (S4, methods) |
| print.c:PrintSpecial:794/801 | print.rs (not implemented error) | no port yet (printing primitives) |
| saveload.c:do_save:2085, do_saveToConn:2420 | none | no port yet (save) |
| methods_list_dispatch.c (5 sites) | none | no port yet (methods package) |
| grDevices devPS.c (3), devQuartz.c, devWindows.c; gevents.c (3) | none | no port yet (graphics devices, events) |

A site ported later calls `eval::eval_promise` (or the hook) on the promise,
not `force`.

## Cases

- tests/rt/eval/promise-eval-depth.R: the headroom at which a promise's code
  runs through each ported path (GNU R's numbers), and UseMethod's retry at a
  depth where both agree (42, one restart warning; main gave two). Its
  `-volume` twin compiles with cmpfun; the small one unserializes GNU R's
  compiled closures (about 1 s under gctorture).
- INTERIM(S1), recorded, not ratcheted:
  - tests/rt/eval/promise-eval-depth-interim.R: the same retry at depths 10-12,
    where GNU R leaves the promise byte code's inlined force was setting up
    PRSEEN 1 (eval.c:setup_bcframe_prom). Closes with item A.
  - tests/rt/jit/base-guard-symbol-value.R: SymbolValue's step (above). Closes
    with X's vm.rs.

## Cost

callgrind, instructions per iteration, against main b84740012: s3 +0.46%,
lapply -0.78%, trycatch +0.05%, the other rows within ±0.1%, vm::run
identical. s3's cost is UseMethod's GetObject now taking eval's step:
force_arg_eval 52 (its frame, R_Visible, the count, the depth step,
R_CheckStack, the flag, ppacct's 6, the call), get_object +16 (GetObject's
unevaluated test, a slot-kind decode), eval_in +6 (the flag). GNU R pays the
same work there; CI's gate is 0.5%.
