# writeBin/readBin with size = sizeof(long double): the x87 80-bit format in
# 16 bytes (the 6-byte tail of connections.c's static long double stays zero),
# swapped as a whole block.
fil <- tempfile("ldbl")
zz <- file(fil, "wb")
writeBin((pi/3)^(1:5), zz, size = 16)
writeBin(c(1, -1, 2^-1074, Inf, NaN), zz, size = 16)
close(zz)
zz <- file(fil, "rb")
print(readBin(zz, numeric(), 5, size = 16))
print(readBin(zz, numeric(), 5, size = 16))
close(zz)
# swapped writes read back through the same whole-block swap
zz <- file(fil, "wb")
writeBin((pi/3)^(1:3), zz, size = 16, endian = "swap")
close(zz)
print(readBin(file(fil, "rb"), numeric(), 3, size = 16, endian = "swap"))
unlink(fil)
