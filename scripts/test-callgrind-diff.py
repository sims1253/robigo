#!/usr/bin/env python3
"""Check scripts/callgrind-diff.py's causes (benchmarks/causes.toml) on real
callgrind_annotate lines, and scripts/idiom-vs-r.py's idiom data."""
import importlib.util
import tomllib
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


cd = module("callgrind_diff", "callgrind-diff.py")
CAUSES = cd.load_causes()
WT = "/home/u/.t3/worktrees/robigo/t3-1"
OBJ = f" [{WT}/.robigo/bin/robigo]"
REG = "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f"
STD = "/rustc/b940084d7eb6a299eb4bfeb8e34901bc051e7ac4/library"

# callgrind_annotate lines (cost columns dropped) and their causes, as
# measured on robigo/fib, rbench/awf/Bounce, rbench/shootout/nbody and
# robigo/vecarith.
CASES = [
    (f"{WT}/crates/robigo-runtime/src/vm.rs:robigo_runtime::vm::run_inline_args'2{OBJ}", "Dispatch and evaluation"),
    (f"{STD}/core/src/mem/mod.rs:<robigo_runtime::vm::Vars>::flush_pending", "Dispatch and evaluation"),
    (f"{WT}/crates/robigo-runtime/src/env.rs:<robigo_runtime::env::Frame>::push_new{OBJ}", "Dispatch and evaluation"),
    (f"{STD}/core/src/ptr/mod.rs:core::ptr::drop_glue::<robigo_runtime::value::Value>{OBJ}",
     "Allocation, reference counts and GC"),
    (f"{REG}/libmimalloc-sys-0.1.49/c_src/mimalloc/v3/src/free.c:mi_free", "Allocation, reference counts and GC"),
    (f"{WT}/crates/robigo-cli/src/main.rs:<alloc::rc::Rc<robigo_runtime::value::Vector<f64>>>::drop_slow",
     "Allocation, reference counts and GC"),
    (f"{WT}/crates/robigo-runtime/src/value.rs:<robigo_runtime::value::Value as core::clone::Clone>::clone{OBJ}",
     "Allocation, reference counts and GC"),
    (f"{STD}/core/src/option.rs:<robigo_runtime::value::Value>::get_attr", "Attribute handling"),
    (f"{WT}/crates/robigo-runtime/src/attrib.rs:robigo_runtime::attrib::get_attrib{OBJ}", "Attribute handling"),
    (f"{STD}/core/src/slice/iter/macros.rs:<robigo_runtime::vm::GuardState>::revalidate", "Guards and type checks"),
    (f"{WT}/crates/robigo-runtime/src/value.rs:robigo_runtime::coerce::as_logical", "Guards and type checks"),
    ("???:0x000000002635f000 [???]", "JIT-generated code"),
    (f"{WT}/crates/robigo-runtime/src/jit/closure.rs:robigo_runtime::jit::closure::h_d_init{OBJ}", "JIT-generated code"),
    (f"{STD}/core/src/ptr/mod.rs:<regalloc2::ion::data_structures::Env<cranelift_codegen::machinst::vcode::VCode>>::run",
     "JIT and bytecode compilation"),
    (f"{WT}/crates/robigo-runtime/src/compile.rs:<robigo_runtime::compile::Compiler>::call{OBJ}",
     "JIT and bytecode compilation"),
    ("???:log1p [/usr/lib/libm.so.6]", "Builtins and numerics"),
    (f"{WT}/crates/robigo-numerics/src/ldouble.rs:robigo_runtime::builtins::summary::rsum", "Builtins and numerics"),
    ("???:0x00000000001b1980 [/usr/lib/libc.so.6]", "libc: memcpy, memset and the rest"),
    (f"{REG}/zlib-rs-0.6.8/src/inflate.rs:zlib_rs::inflate::inflate{OBJ}", "Startup, lazy loading and parsing"),
    (f"{WT}/crates/robigo-runtime/src/value.rs:<robigo_runtime::value::Value>::len{OBJ}",
     "Values (lengths, element access, boxing)"),
    (f"{STD}/core/src/hash/sip.rs:<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write",
     cd.OTHER),
]


class Causes(unittest.TestCase):
    def test_lines_map_to_their_causes(self):
        for line, want in CASES:
            with self.subTest(line=line):
                self.assertEqual(cd.cause_of(cd.short_location(line), CAUSES), want)

    def test_short_location(self):
        self.assertEqual(cd.short_location(CASES[0][0]),
                         "crates/robigo-runtime/src/vm.rs:robigo_runtime::vm::run_inline_args [robigo]")
        self.assertEqual(cd.short_location(f"{REG}/zlib-rs-0.6.8/src/inflate.rs:zlib_rs::inflate::inflate"),
                         "zlib-rs-0.6.8/src/inflate.rs:zlib_rs::inflate::inflate")
        self.assertEqual(cd.function_of(cd.short_location(CASES[0][0])), "robigo_runtime::vm::run_inline_args")

    def test_parse_annotate(self):
        text = ("51,417,986 (100.0%)  PROGRAM TOTALS\n"
                f"1,438,066 ( 2.80%)  {CASES[0][0]}\n"
                f"  100 ( 0.00%)  {CASES[0][0].replace(chr(39) + '2', '')}\n")
        total, costs = cd.parse_annotate(text)
        self.assertEqual(total, 51_417_986)
        self.assertEqual(costs, {"crates/robigo-runtime/src/vm.rs:robigo_runtime::vm::run_inline_args [robigo]":
                                 1_438_166})

    def test_table_adds_up(self):
        diff = {cd.short_location(line): 1000 * (k + 1) for k, (line, _) in enumerate(CASES)}
        total = sum(diff.values())
        table = cd.cause_table(diff, total, 1, CAUSES)
        self.assertIn("| Dispatch and evaluation | 6,000 |", table)  # the first three lines: 1000 + 2000 + 3000
        self.assertIn(f"| **Total** | **{total:,}** |", table)
        shown = sum(int(row.split("|")[2].replace(",", "")) for row in table.splitlines()[2:-1])
        self.assertEqual(shown, total)

    def test_every_cause_is_used(self):
        used = {want for _, want in CASES}
        for name, _ in CAUSES:
            if name not in ("Printing, formatting and deparsing", "Other runtime"):
                self.assertIn(name, used, f"no case for {name}")


class Idioms(unittest.TestCase):
    def test_every_idiom_has_a_floor_and_its_arithmetic(self):
        with open(HERE.parent / "benchmarks" / "idioms.toml", "rb") as f:
            idioms = tomllib.load(f)["idiom"]
        self.assertTrue(idioms)
        names = [d["name"] for d in idioms]
        self.assertEqual(len(names), len(set(names)))
        for d in idioms:
            with self.subTest(idiom=d["name"]):
                self.assertGreater(d["c"]["floor"], 0)
                self.assertTrue(d["c"]["arithmetic"])
                # the C version parses N: N and 2N must have as many digits
                self.assertEqual(len(str(d["n"])), len(str(2 * d["n"])))
                self.assertIn("for (int i = 1; i <= n; i++)", module("idiom", "idiom-vs-r.py").c_source(d))


if __name__ == "__main__":
    unittest.main()
