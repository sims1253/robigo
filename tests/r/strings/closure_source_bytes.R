options(keep.parse.data=FALSE)
for (text in c('function() "\\304"', 'function() "é"', 'function() "\\303\\251"')) {
    f <- eval(parse(text=text, keep.source=TRUE))
    print(f)
    print(f, useSource=FALSE)
    print(unserialize(serialize(f,NULL)))
    print(charToRaw(f()))
}
