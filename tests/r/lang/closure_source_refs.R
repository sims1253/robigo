options(keep.parse.data = FALSE)
p <- parse(text = 'function(a = function() 1) {\n  function(x) x + a\n}', keep.source = TRUE)
f <- eval(p)
ref <- p[[1]][[4]]
print(identical(attr(f, 'srcref'), ref))
print(as.integer(attr(f, 'srcref')))
print(names(attributes(f)))
print(identical(attr(attr(f, 'srcref'), 'srcfile'), attr(p, 'srcfile')))
print(identical(attr(body(f), 'srcref'), attr(p[[1]][[3]], 'srcref')))
g <- f()
print(as.integer(attr(g, 'srcref')))
print(as.integer(attr(eval(formals(f)$a), 'srcref')))
comment(f) <- 'closure metadata'
for (version in 2:3) {
    copy <- unserialize(serialize(f, NULL, version = version))
    print(comment(copy))
    print(as.integer(attr(copy, 'srcref')))
    print(identical(attr(copy, 'srcref'), attr(f, 'srcref')))
    print(identical(attr(attr(copy, 'srcref'), 'srcfile'), attr(body(copy), 'srcfile')))
    print(copy(3)(2))
}
plain <- eval(parse(text = 'function(x) x', keep.source = FALSE))
print(attributes(plain))
call <- quote(function(x) x)
call[[4]] <- 1:8
print(attr(eval(call), 'srcref'))
attr(call, 'comment') <- 'call comment'
print(comment(eval(call)))
print(as.character(attr(f, 'srcref')))
print(attributes(args(f)))
