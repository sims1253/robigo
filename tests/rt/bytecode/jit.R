# eval.c's enableJIT and compilePKGS: each returns the old setting;
# enableJIT's level of 0 or more is set (a negative one only queries),
# after loading the compiler and its checkCompilerOptions; compilePKGS
# loads the compiler when set to TRUE. (The level is set first: the run
# that records what base reaches runs with the JIT off.)
invisible(.Internal(enableJIT(3)))
print(.Internal(enableJIT(-1)))
print(.Internal(enableJIT(0)))
print(.Internal(enableJIT(-1)))
print(.Internal(enableJIT(3)))
print(.Internal(enableJIT(-1)))
print(.Internal(compilePKGS(FALSE)))
print(.Internal(compilePKGS(TRUE)))
print(.Internal(compilePKGS(NA)))
print(.Internal(compilePKGS(FALSE)))
print(compiler::enableJIT(-1))
print(compiler::compilePKGS(FALSE))
print(isNamespace(asNamespace("compiler")))
print(compiler::getCompilerOption("optimize"))
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
print(err(compiler::nonexistent))
print(environmentName(topenv(environment(compiler::cmpfun))))
