"""Generated R programs aimed at what the VM and the JIT assume.

    program(rnd) -> str

Every program is valid R built from templates (after Csmith and Blood's
tools/codegen_fuzz.py): a hot loop, or a closure called often enough to be
compiled, whose body changes something the compiled code may have
specialized on, after the code is hot. The loop JIT compiles a loop after
16 iterations (vm.rs JIT_THRESHOLD), the closure JIT a body after 512 calls
(jit/closure.rs THRESHOLD), so the switch iteration is usually beyond both.
What changes:

- a variable's type, length or attributes mid-loop;
- `<<-` from a closure, `assign()`/`eval()`/`rm()`/`delayedAssign()` into
  the loop's own frame, an active binding appearing;
- integer overflow to NA (with its warning), NA reaching a condition;
- `[<-`/`[[<-` on a vector another variable shares (copy-on-modify);
- an S3 method, a callee, or a base function shadowed after warm-up.

Programs print their state, so GNU R (scripts/fuzz-eval.py) or robigo's
other tiers (--tiers) decide what is right.
"""
import random

INITS = ["0L", "0", "1L", "0.5", ".Machine$integer.max - 5L", "-2147483640L", "FALSE", "c(a = 1L)",
         "NA_integer_", "1i", "integer(0)"]

# {at}: the condition true once, after warm-up (i == k, or calls == k).
STEADY = ["acc <- acc + i", "acc <- acc + 1L", "acc <- acc * 2L", "acc <- acc - i / 2", "acc <- acc %/% 2L + i",
          "acc <- acc %% 7L", "cnt <- cnt + 1L", "acc <- -acc", "acc <- acc * acc",
          "if (isTRUE(acc > 10)) cnt <- cnt + 1L", "if (is.na(acc)) cnt <- cnt + 1L",
          "if (i %% 7L == 0L) next", "cnt <- cnt + .Machine$integer.max %/% 3L"]
SWITCH = ["if ({at}) acc <- as.double(acc)", "if ({at}) acc <- as.integer(acc)", "if ({at}) acc <- acc + 0i",
          "if ({at}) acc <- as.logical(acc)", "if ({at}) acc <- c(acc, acc)", "if ({at}) names(acc) <- \"a\"",
          "if ({at}) class(acc) <- \"cls\"", "if ({at}) storage.mode(acc) <- \"double\"", "if ({at}) acc <- NA",
          "if ({at}) acc <- NULL", "if ({at}) acc <- \"s\"", "if ({at}) {{ rm(acc); acc <- 0.5 }}",
          "if ({at}) delayedAssign(\"acc\", i * 2)", "if ({at}) warning(\"switched at \", i)",
          "if ({at}) break"]
ENVS = ["assign(\"acc\", acc + 1L)", "eval(quote(acc <- acc + i))", "evalq(cnt <- cnt + 1L, environment())",
        "local(cnt <<- cnt + 2L)", "bump(i)", "if ({at}) bump <- function(d) acc <<- acc - d",
        "if ({at}) makeActiveBinding(\"ab\", function() i, environment())\n    if (exists(\"ab\", inherits = FALSE)) cnt <- cnt + ab",
        "acc <- get(\"acc\") + 1L", "if ({at}) environment(bump) <- new.env()"]
VECTORS = ["x[i %% 3L + 1L] <- i", "x[[i %% 3L + 1L]] <- i / 2", "if ({at}) x[5] <- 9L", "z <- x; z[1] <- -1L",
           "x <- g(x)", "l[[1]][1] <- i", "attr(x, \"it\") <- i", "if ({at}) x <- as.character(x)",
           "if ({at}) dim(x) <- NULL", "x[\"n\"] <- cnt"]
METHODS = ["cnt <- cnt + length(o)", "if ({at}) length.cls <- function(x) 10L",
           "acc <- acc + unclass(o2 + 1L)", "if ({at}) `+.num2` <- function(e1, e2) structure(7L, class = \"num2\")",
           "acc <- h(acc)", "if ({at}) h <- function(a) a * 2L", "cnt <- cnt + abs(-i)",
           "if ({at}) abs <- function(x) 0L", "if ({at}) `+` <- function(e1, e2) base::`-`(e1, e2)",
           "acc <- acc + o[[\"v\"]]", "if ({at}) `[[.cls` <- function(x, i) 100L",
           "if ({at}) length <- function(x) -1L"]
SETUP = """acc <- {init}
    cnt <- 0L
    x <- c(1L, 2L, 3L)
    y <- x
    l <- list(x)
    g <- function(v) {{ v[1] <- v[1] + 1L; v }}
    h <- function(a) a + 1L
    bump <- function(d) acc <<- acc + d
    o <- structure(list(v = 1), class = "cls")
    o2 <- structure(1L, class = "num2")"""
REPORT = """print(acc)
    print(cnt)
    print(x)
    print(y)
    print(identical(x, y))
    print(l)"""


def loop(rnd, body):
    kind = rnd.randrange(5)
    inner = "\n".join("        " + line for line in body)
    if kind == 0:
        return f"for (i in seq_len(n)) {{\n{inner}\n    }}"
    if kind == 1:
        return f"for (i in rev(seq_len(n))) {{\n{inner}\n    }}"
    if kind == 2:
        return f"for (i in seq(1, n, by = 1)) {{\n{inner}\n    }}"
    if kind == 3:
        return f"i <- 0L\n    while (i < n) {{\n        i <- i + 1L\n{inner}\n    }}"
    return f"i <- 0L\n    repeat {{\n        i <- i + 1L\n        if (i > n) break\n{inner}\n    }}"


def program(rnd):
    """One program: setup, a hot loop or hot closure, a report."""
    hot = rnd.random() < 0.3
    n = rnd.choice([20, 40, 100, 300]) if not hot else rnd.choice([2, 3])
    k = rnd.randint(17, n + 5) if not hot else rnd.randint(500, 650)
    at = f"i == {k}" if not hot else f"calls == {k}L"
    body = []
    for _ in range(rnd.randint(2, 5)):
        pool = rnd.choice([STEADY, SWITCH, ENVS, VECTORS, METHODS])
        body.append(rnd.choice(pool).format(at=at))
    rnd.shuffle(body)
    setup = SETUP.format(init=rnd.choice(INITS))
    if hot:
        calls = rnd.choice([600, 700])
        return (f"calls <- 0L\nf <- function(n) {{\n    calls <<- calls + 1L\n    {setup}\n    {loop(rnd, body)}\n"
                f"    if (calls %% 100L == 0L) {{\n    {REPORT}\n    }}\n    acc\n}}\n"
                f"res <- NULL\nfor (r in seq_len({calls})) res <- f({n}L)\nprint(res)\n")
    if rnd.random() < 0.6:
        n2 = rnd.choice([n, n // 2, 1, 0])
        return (f"f <- function(n) {{\n    {setup}\n    {loop(rnd, body)}\n    {REPORT}\n    invisible(acc)\n}}\n"
                f"f({n}L)\nf({n2}L)\nprint(f({n}L))\n")
    return f"n <- {n}L\n{setup}\n{loop(rnd, body)}\n{REPORT}\n".replace("\n    ", "\n")


if __name__ == "__main__":
    import sys
    print(program(random.Random(int(sys.argv[1]) if len(sys.argv) > 1 else 0)))
