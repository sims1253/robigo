for (src in c('x + 1', 'x = x + 1', '1 -> x ->> y', 'a <- b <- 1',
              'f(,x,,a=,NULL=1,"b"=2)', 'pkg::fun(x@slot)',
              'function(x, y=1, ...) { x+y }', '# before\nx # after\n# tail',
              '{ if(x) y\nz }', 'x |> f()', 'x\t+\ty', '"é" + café')) {
    p <- parse(text=src, keep.source=TRUE)
    data <- attr(p, 'srcfile')$parseData
    print(unclass(data))
    print(utils::getParseData(p))
}
print(utils::getParseData(parse(text='x + 1', keep.source=FALSE)))
p <- parse(text='x; y; z', n=2, keep.source=TRUE)
print(utils::getParseData(p))
options(keep.parse.data=FALSE)
p <- parse(text='x', keep.source=TRUE)
print(utils::getParseData(p))
