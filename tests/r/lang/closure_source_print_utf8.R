options(keep.parse.data = FALSE)
g <- eval(parse(text = 'function(x) {\n\t"é"; x\n}', keep.source = TRUE))
print(g)
print(g, useSource = FALSE)
