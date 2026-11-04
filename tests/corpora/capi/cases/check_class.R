dyn.load("check_class.so")

# S3: the first class is looked up, the others are not
print(.Call("capi_check_class", structure(1, class = "foo")))
print(.Call("capi_check_class", structure(1, class = c("foo", "bar"))))
print(.Call("capi_check_class", structure(1, class = c("bar", "foo"))))
print(.Call("capi_check_class", structure(list(), class = "baz")))
# not an object (no class attribute): -1 even for a valid implicit class
print(.Call("capi_check_class", 1))
print(.Call("capi_check_class", matrix(1)))

# S4: a direct match, then through a non-virtual superclass
methods::setClass("Base", methods::representation(x = "numeric"))
methods::setClass("Derived", contains = "Base")
methods::setClass("Deeper", contains = "Derived")
methods::setClass("V", methods::representation("VIRTUAL"))
methods::setClass("WithV", contains = c("V", "Base"))
methods::setClass("Num", contains = "numeric")
methods::setClass("Lone", methods::representation(y = "character"))
print(.Call("capi_check_class", methods::new("Base", x = 1)))
# .selectSuperClasses is called in the class's environment (here the
# global one), so it is not found while methods is not attached
print(tryCatch(.Call("capi_check_class", methods::new("Derived", x = 1)),
               error = conditionMessage))
library(methods)
print(.Call("capi_check_class", methods::new("Derived", x = 1)))
print(.Call("capi_check_class", methods::new("Deeper", x = 1)))
# a virtual superclass does not count
print(.Call("capi_check_class", methods::new("WithV", x = 1)))
print(.Call("capi_check_class", methods::new("Num", 1:2 + 0.5)))
print(.Call("capi_check_class", methods::new("Lone", y = "a")))
# an S4 object of a class from a package (methods)
print(.Call("capi_check_class", methods::getClass("Base")))

# R_data_class
for (x in list(1, 1L, "a", TRUE, NULL, list(), matrix(1:4, 2), array(1:8, c(2, 2, 2)),
               array(1, 1), sum, function(x) x, quote(f(x)), quote(if (a) b),
               quote(x <- 1), quote((x)), quote({}), quote(for (i in 1) 1),
               quote(while (TRUE) 1), call("=", quote(a), 1), quote(x), new.env(),
               structure(1, class = c("a", "b")), structure(matrix(1), class = "m"),
               methods::new("Base", x = 1), methods::new("Num", 1),
               methods::getClass("Base")@prototype, expression(1), 1i, as.raw(1)))
    print(.Call("capi_data_class", x))
