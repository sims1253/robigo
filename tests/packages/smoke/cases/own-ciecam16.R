# needs: ciecam16
# The user's own package (github.com/sims1253/ciecam16): the CIECAM16
# colour appearance model. Its README builds two specifications and
# calls `==`, as.list() and as.data.frame() on them. Those methods are
# registered in a local() block when the package is built, with no
# S7::methods_register() in .onLoad, so the installed package has none of
# them: GNU R 4.5.0 stops at `==` ("Can't find method for generic") and
# at as.list() ("cannot coerce type 'closure'"). This keeps the README's
# two specifications and reads them through what the package does export:
# the class, its properties and S7's default print.
suppressPackageStartupMessages(library(ciecam16))
color1 <- CAM16_Specification(J = 50, C = 30, h = 180, H = 200, e = 0.9)
color2 <- CAM16_Specification(J = 60, C = 40, h = 200, H = 220, e = 0.8)
print(identical(color1, color2))
print(c(J = color1@J, C = color1@C, h = color1@h, H = color1@H, e = color1@e))
print(S7::S7_inherits(color2, CAM16_Specification))
color2
