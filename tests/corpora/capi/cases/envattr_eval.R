dyn.load("envattr_eval.so")

print(.Call("capi_mask_eval", quote(class(.env))))
print(.Call("capi_mask_eval", quote(inherits(.env, "pronoun"))))

e <- new.env()
print(.Call("capi_attr_eval", e, quote(attr(e, "tag"))))
print(attr(e, "tag"))

class(e) <- "k"
print(.Call("capi_unclass_eval", e, quote(class(e))))
print(class(e))

s <- new.env()
print(.Call("capi_s4_eval", s, quote(isS4(s))))
print(isS4(s))
