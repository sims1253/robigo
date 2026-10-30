#!/usr/bin/env python3
"""Extract GNU R 4.5.0's metadata parser tables; run from the repo root.

Usage: python3 crates/robigo-syntax/src/parse_data/generate.py PATH/TO/gram.c
Then run cargo fmt. The generated Rust has no dependency on bison or GNU R.
"""
import json
import pathlib
import re
import sys

source = pathlib.Path(sys.argv[1]).read_text()
output = [
    "// Generated from GNU R 4.5.0 src/main/gram.c (GPL-2.0-or-later).",
    "// R: gram.y, Bison 3.8.2 parser tables. Do not edit by hand.",
]
for name in ("yytranslate", "yypact", "yydefact", "yypgoto", "yydefgoto",
             "yytable", "yycheck", "yyr1", "yyr2"):
    array = re.search(r"\b" + name + r"\[\]\s*=\s*\{(.*?)\};", source, re.S)[1]
    numbers = re.findall(r"-?\d+", array)
    output.append(f"pub const {name.upper()}: &[i16] = &[\n" + "".join(
        "    " + ", ".join(numbers[i:i+16]) + ",\n"
        for i in range(0, len(numbers), 16)) + "];")
array = re.search(r"yytname\[\]\s*=\s*\{(.*?)\};", source, re.S)[1]
names = [json.loads(x) for x in re.findall(r'"(?:\\.|[^"\\])*"', array)]
output.append("pub const NAMES: &[&str] = &[" + ", ".join(map(json.dumps, names)) + "];")
for name in ("YYPACT_NINF", "YYNTOKENS", "YYLAST"):
    number = re.search(r"#define " + name + r"\s+\(?(-?\d+)", source)[1]
    output.append(f"pub const {name}: i16 = {number};")
actions = re.findall(r"case (\d+): /\* (.*?) \*/\s*(.*?)\s*break;", source, re.S)
external = {key: int(value) for key, value in re.findall(r"^\s+(\w+) = (\d+)", source, re.M)}
recorded, modifications = [], []
for number, rule, action in actions:
    if int(number) > 94:
        continue
    if "setId" in action:
        recorded.append(number)
    for offset, token in re.findall(
        r"modif_token\(\s*&\(yylsp\[(-?\d+)\]\),\s*(\w+)\s*\)", action
    ):
        modifications.append(f"({number}, {offset}, {external[token]})")
output.append("pub const RECORDED_RULES: &[usize] = &[" + ", ".join(recorded) + "];")
output.append("pub const MODIFICATIONS: &[(usize, isize, i16)] = &[" + ", ".join(modifications) + "];")
pathlib.Path(__file__).with_name("tables.rs").write_text("\n".join(output) + "\n")
