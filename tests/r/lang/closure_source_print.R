options(keep.parse.data = FALSE)
f <- eval(parse(text = 'function( x = 1 ) { # body comment\n    x  +  2\n}', keep.source = TRUE))
print(f)
print(f, useSource = FALSE)
print(list(f), useSource = TRUE)
for (version in 2:3) {
    copy <- unserialize(serialize(f, NULL, version = version))
    print(copy)
}
print(f, useSource = NA)
