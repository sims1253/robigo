# scripts/callgrind-diff.py, s3 row (N=8000 against 4000), per iteration, top 40: main cc5d091e, then 754f0146

## main

```
total: work 196,358,605  base 119,140,955  per unit 19,304
     1,400.0    7.3%  robigo_rt::vm::run
     1,370.0    7.1%  robigo_rt::dispatch::dispatch_method
       940.0    4.9%  robigo_rt::args::apply_with::<false>::{closure#0}
       890.0    4.6%  robigo_rt::dispatch::usemethod
       772.0    4.0%  <robigo_rtcore::isolate::Capability<[()]>>::scope::<core::result::Result<(), robigo_rtcore::exit::RError>, <robigo_rtcore::isolate::Capabili
       763.0    4.0%  <robigo_heap::gc::Core>::alloc_node
       677.0    3.5%  <robigo_rtcore::isolate::Capability<[()]>>::new_closure_frame
       517.0    2.7%  robigo_rt::dispatch::memo_get
       516.0    2.7%  robigo_rt::dispatch::topenv
       492.0    2.5%  <robigo_rtcore::isolate::Capability<[()]>>::cleanup_envir
       469.0    2.4%  robigo_rt::dispatch::do_usemethod
       455.0    2.4%  <robigo_rtcore::isolate::Capability<[()]>>::cleanup_envir::{closure#1}
       438.0    2.3%  <robigo_rtcore::isolate::Capability<[()]>>::bind_frame_slots
       372.0    1.9%  robigo_rtbuiltins::subset::subset3_dflt
       363.0    1.9%  robigo_rt::dispatch::quick_search
       346.0    1.8%  <robigo_rtcore::isolate::Capability<[()]>>::scope::<core::result::Result<core::option::Option<robigo_heap::Sexp>, robigo_rtcore::exit::RErro
       288.0    1.5%  robigo_rt::vm::exec
       276.0    1.4%  robigo_rt::args::generic_args
       242.0    1.3%  robigo_rt::eval::eval_in
       242.0    1.3%  <robigo_heap::gc::Core>::alloc_vector_slow
       237.0    1.2%  <robigo_rtcore::isolate::Capability<[()]>>::deep_cells::{closure#0}
       232.0    1.2%  <robigo_rtcore::isolate::Capability<[()]>>::function_context::<robigo_rt::dispatch::dispatch_special::{closure#0}::{closure#5}>
       231.0    1.2%  free
       230.0    1.2%  robigo_rt::dispatch::promise_call_args
       225.0    1.2%  robigo_rt::args::generic_source
       216.0    1.1%  robigo_rt::dispatch::data_class2
       215.0    1.1%  <robigo_rtcore::isolate::Capability<[()]>>::force_arg
       213.0    1.1%  <robigo_heap::gc::Core>::alloc_slot_frame
       211.0    1.1%  robigo_rt::dispatch::get_object
       210.0    1.1%  <robigo_rtcore::isolate::Capability<[()]>>::ensure_object
       199.0    1.0%  robigo_rtcore::env::promise::clear_promise
       186.0    1.0%  malloc
       182.0    0.9%  robigo_rtcore::env::loc_in_frame
       173.0    0.9%  robigo_rt::bcassign::dollar_call
       173.0    0.9%  <robigo_rtcore::isolate::Capability<[()]>>::recycle_frame
       159.0    0.8%  <robigo_heap::gc::Core>::alloc_vector
       151.0    0.8%  robigo_rtcore::env::promise::release_value
       146.0    0.8%  <robigo_rtcore::isolate::Capability<[()]>>::end_call
       138.0    0.7%  robigo_rt::dispatch::same_dispatch
       129.4    0.7%  <robigo_heap::gc::Core as core::ops::drop::Drop>::drop
```

## 754f0146

```
total: work 191,920,620  base 116,922,944  per unit 18,749
     1,403.0    7.5%  robigo_rt::vm::run
     1,364.0    7.3%  robigo_rt::dispatch::dispatch_method
       940.0    5.0%  robigo_rt::args::apply_with::<false>::{closure#0}
       890.0    4.7%  robigo_rt::dispatch::usemethod
       772.0    4.1%  <robigo_rtcore::isolate::Capability<[()]>>::scope::<core::result::Result<(), robigo_rtcore::exit::RError>, <robigo_rtcore::isolate::Capabili
       699.0    3.7%  <robigo_heap::gc::Core>::alloc_node
       697.0    3.7%  <robigo_rtcore::isolate::Capability<[()]>>::new_closure_frame
       517.0    2.8%  robigo_rt::dispatch::memo_get
       516.0    2.8%  robigo_rt::dispatch::topenv
       492.0    2.6%  <robigo_rtcore::isolate::Capability<[()]>>::cleanup_envir
       469.0    2.5%  robigo_rt::dispatch::do_usemethod
       438.0    2.3%  <robigo_rtcore::isolate::Capability<[()]>>::bind_frame_slots
       399.0    2.1%  <robigo_rtcore::isolate::Capability<[()]>>::cleanup_envir::{closure#1}
       372.0    2.0%  robigo_rtbuiltins::subset::subset3_dflt
       363.0    1.9%  robigo_rt::dispatch::quick_search
       346.0    1.8%  <robigo_rtcore::isolate::Capability<[()]>>::scope::<core::result::Result<core::option::Option<robigo_heap::Sexp>, robigo_rtcore::exit::RErro
       288.0    1.5%  robigo_rt::vm::exec
       276.0    1.5%  robigo_rt::args::generic_args
       242.0    1.3%  robigo_rt::eval::eval_in
       242.0    1.3%  <robigo_heap::gc::Core>::alloc_vector_slow
       237.0    1.3%  <robigo_rtcore::isolate::Capability<[()]>>::deep_cells::{closure#0}
       232.0    1.2%  <robigo_rtcore::isolate::Capability<[()]>>::function_context::<robigo_rt::dispatch::dispatch_special::{closure#0}::{closure#5}>
       231.0    1.2%  free
       230.0    1.2%  robigo_rt::dispatch::promise_call_args
       216.0    1.2%  robigo_rt::dispatch::data_class2
       214.0    1.1%  <robigo_rtcore::isolate::Capability<[()]>>::force_arg
       213.0    1.1%  <robigo_heap::gc::Core>::alloc_slot_frame
       211.0    1.1%  robigo_rt::dispatch::get_object
       186.0    1.0%  malloc
       182.0    1.0%  robigo_rtcore::env::loc_in_frame
       173.0    0.9%  robigo_rt::bcassign::dollar_call
       173.0    0.9%  <robigo_rtcore::isolate::Capability<[()]>>::recycle_frame
       159.0    0.8%  <robigo_heap::gc::Core>::alloc_vector
       141.0    0.8%  robigo_rtcore::env::promise::clear_promise
       138.0    0.7%  robigo_rt::dispatch::same_dispatch
       129.0    0.7%  robigo_rt::args::method_plan_get
       126.0    0.7%  robigo_rtcore::env::frame::next_stamp
       121.9    0.7%  <robigo_heap::gc::Core as core::ops::drop::Drop>::drop
       117.0    0.6%  robigo_rt::dispatch::method_shape
       117.0    0.6%  robigo_rtcore::env::promise::release_value
```

# fib, inline::enter, against main 17ed87e9 (callgrind-diff.py, per fib(18), 20 against 40)

## main-17ed
```
total: work 468,913,376  base 254,462,225  per unit 10,722,558
 7,745,166.5   72.2%  robigo_rt::inline::enter
 1,744,347.8   16.3%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::eval
   515,706.9    4.8%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::read
   148,311.8    1.4%  robigo_rt::vm::run
   139,086.9    1.3%  robigo_rt::args::apply_with::<false>::{closure#0}
```
## alias-p
```
total: work 471,060,681  base 255,535,868  per unit 10,776,241
 7,798,995.9   72.4%  robigo_rt::inline::enter
 1,744,347.8   16.2%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::eval
   515,706.9    4.8%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::read
   148,025.0    1.4%  robigo_rt::vm::run
   138,943.5    1.3%  robigo_rt::args::apply_with::<false>::{closure#0}
```
## alias-v
```
total: work 468,921,116  base 254,467,164  per unit 10,722,698
 7,745,166.5   72.2%  robigo_rt::inline::enter
 1,744,347.8   16.3%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::eval
   515,706.9    4.8%  <robigo_rt::inline::Frame as robigo_rt::inline::Scope>::read
   148,312.8    1.4%  robigo_rt::vm::run
   138,943.5    1.3%  robigo_rt::args::apply_with::<false>::{closure#0}
```
