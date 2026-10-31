# Small internals: arity and error paths from platform.c (unlink),
# unique.c (rowsum's HashTableSetup), errors.c (gettext/ngettext/
# bindtextdomain), array.c (lengths, max.col's random ties), sys-unix.c
# (system timeout), sysutils.c (setSessionTimeLimit), envir.c
# (lockEnvironment, parent.env) and builtin.c (formals).
tried <- function(e) print(tryCatch(eval(e), error = function(x) conditionMessage(x)))

# unlink requires all four arguments even when x is empty.
tried(quote(.Internal(unlink("/nonexistent", TRUE, FALSE))))
tried(quote(.Internal(unlink(character(0), FALSE, FALSE))))
print(unlink(tempfile(), recursive = TRUE, force = FALSE, expand = FALSE))

# rowsum's hash table takes vectors but not NULL or a closure.
tried(quote(.Internal(rowsum_matrix(matrix(1:4, 2), c(1L, 2L), NULL, FALSE, "1"))))
tried(quote(.Internal(rowsum_matrix(matrix(1:4, 2), c(1L, 2L), print, FALSE, "1"))))

# gettext returns a NULL or empty string as is; bindtextdomain answers
# with the directory it was given.
print(.Internal(gettext(NULL, NULL)))
print(.Internal(gettext(NULL, character(0))))
print(.Internal(gettext(NULL, "abc")))
print(.Internal(bindtextdomain("utils", "/tmp/translations")))
print(.Internal(bindtextdomain(NULL, NULL)))
tried(quote(.Internal(bindtextdomain(c("a", "b"), "/x"))))
tried(quote(.Internal(bindtextdomain("a", c("x", "y")))))

# ngettext coerces n with asInteger (imaginary parts warn) and says
# "invalid 'n' argument".
print(ngettext(1 + 1i, "a", "b"))
print(ngettext(2, "one", "many"))
tried(quote(ngettext(-1, "a", "b")))
tried(quote(ngettext(NA_integer_, "a", "b")))

# lengths validates use.names and reports the call.
tried(quote(lengths(quote(a))))
tried(quote(lengths(42, use.names = "as")))
print(lengths(1:3))
print(lengths(list(1, 1:2), use.names = TRUE))

# ngettext's asInteger only converts the first element; a list is NA.
print(ngettext(c(1 + 0i, 2 + 1i), "one", "many"))
tried(quote(ngettext(list(1), "a", "b")))

# max.col breaks ties at random with the usual RNG stream.
set.seed(1)
print(max.col(matrix(c(1, 1, 0, 0, 1, 1), 2), "random"))
print(max.col(rbind(c(2, 2, 2), c(1, 2, 3)), "random"))
set.seed(42)
print(max.col(matrix(rep(1, 12), 3), "random"))

# system(timeout=) kills the child, warns and answers 124.
t <- proc.time()[["elapsed"]]
print(system("sleep 2", timeout = 1))
print(proc.time()[["elapsed"]] - t < 2)
print(system("true", timeout = 5))
print(system("exit 3", timeout = 5))
# Captured output survives a timeout and carries status 124; the whole
# process group is killed, so `survived` never prints.
r <- system("printf ok; sleep 2", intern = TRUE, timeout = 1)
print(r)
print(attr(r, "status"))
print(system("(sleep 1; echo survived)", timeout = 1))
print(length(system("seq 1 20000", intern = TRUE, timeout = 5)))
# A child that finishes before the deadline keeps its status.
for (i in 1:4) print(system("sleep 0.5", timeout = 1))
# A background descendant holding the pipe open does not extend the
# deadline past the shell's exit.
r <- system("sleep 3 & true", intern = TRUE, timeout = 1)
print(r)
print(attr(r, "status"))

# setSessionTimeLimit takes two numbers and returns NULL.
print(.Internal(setSessionTimeLimit(NULL, NULL)))
print(.Internal(setSessionTimeLimit(FALSE, Inf)))
print(setSessionTimeLimit(cpu = Inf, elapsed = Inf))

# lockEnvironment reviews bindings before the environment, and parent.env
# reports its own call.
e <- new.env()
tried(quote(lockEnvironment(e, logical())))
tried(quote(lockEnvironment("foo", TRUE)))
tried(quote(lockEnvironment(e, c(TRUE, FALSE))))
tried(quote(lockEnvironment(e, list(TRUE))))
tried(quote(lockEnvironment(NULL, TRUE)))
tried(quote(lockEnvironment(NULL, logical())))
tried(quote(lockEnvironment("foo", TRUE)))
print(lockEnvironment(e, TRUE))
print(environmentIsLocked(e))
tried(quote(parent.env(1)))
tried(quote(parent.env(NULL)))
print(identical(parent.env(e), globalenv()))

# formals warns for non-functions, like body().
print(formals(1))
print(formals(function(a, b = 2) a + b))
