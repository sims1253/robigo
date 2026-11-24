# round() and signif() compile to CALLSPECIAL (a special, in cmp.R's foldFuns
# but not folded); the compiled call runs do_Math2 and dispatches
compiler::disassemble(compiler::compile(quote(round(2.567, 2))))
compiler::disassemble(compiler::compile(quote(signif(123456, 2) + x)))
compiler::disassemble(compiler::compile(quote(round(1+2i))))
compiler::disassemble(compiler::compile(quote(round(digits = 2))))
compiler::disassemble(compiler::compile(quote(round(x, 2))))
f <- compiler::cmpfun(function(x) round(x, 1:2)); f(c(1.234, 5.678)); f(structure(1.5, class = "foo"))
