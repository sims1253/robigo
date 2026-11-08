# needs: ciecam16
# The user's own package (github.com/sims1253/ciecam16): the CIECAM16
# colour appearance model. Its README example: two specifications,
# compared and converted.
suppressPackageStartupMessages(library(ciecam16))
color1 <- CAM16_Specification(J = 50, C = 30, h = 180, H = 200, e = 0.9)
color2 <- CAM16_Specification(J = 60, C = 40, h = 200, H = 220, e = 0.8)
print(color1 == color2)
print(as.list(color1))
print(as.data.frame(color2))
