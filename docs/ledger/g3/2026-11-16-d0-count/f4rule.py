"""What D0's count subtracts as already Rust at M3 (lead decision F4: A),
besides M1's registry rows."""

# 2. Evaluator-owned: rows whose C function is defined in the files g3.md §10
#    names (Rust from M3).
EVAL_FILES = {"eval.c", "envir.c", "errors.c", "context.c", "debug.c", "memory.c", "objects.c", "match.c", "main.c"}
# 3. Designed away (g3.md §10's first bullet).
#  (a) The language specials, VM ops: §10's examples and the rest of
#      cmp.R's language handlers (if, while, for, repeat, break, next,
#      return, function, <-, =, <<-, {, &&, ||, switch, .Internal, ::,
#      :::), with `(`, quote, substitute, missing, on.exit, UseMethod and
#      NextMethod.
LANG = {"if", "while", "for", "repeat", "break", "next", "return", "function", "<-", "=", "<<-", "{", "(",
        ".Internal", "quote", "substitute", "missing", "on.exit", "switch", "&&", "||", "UseMethod",
        "NextMethod", "::", ":::"}
#  (b) The kernels the VM and JIT own, by C function: arithmetic, comparison,
#      logic (`&`, `|`, `!`, all, any), math1/2 (names.c's "Mathematical
#      Functions" families: math1, trunc, abs, log, log2/log10, round/signif,
#      math2), summaries (the Summary group and mean: do_summary, do_range),
#      `:`, seq_len, seq_along, length, is.* and as.* (coerce.c's type
#      checks and atomic coercions), c, list, rep.
KERNEL_CFUNS = {
    "do_arith", "do_relop", "do_logic", "do_logic3",
    "do_math1", "do_trunc", "do_abs", "do_log", "do_log1arg", "do_Math2", "do_math2",
    "do_summary", "do_range",
    "do_colon", "do_seq_len", "do_seq_along", "do_length",
    "do_is", "do_isna", "do_isnan", "do_isfinite", "do_isinfinite", "do_isvector",
    "do_asatomic", "do_asvector",
    "do_c", "do_makelist", "do_rep", "do_rep_int", "do_rep_len",
}


