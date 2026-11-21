Byte-exact inputs for the readLines and pushBack cases (robigo-rt has no
`writeBin` yet to make them): CR and CRLF line ends, an embedded NUL, a
UTF-8 BOM, an incomplete final line, an empty file.
`nul-c3.bin` is a NUL and then the first byte of a two-byte UTF-8
character (readChar's NUL warning comes before the invalid UTF-8 error).
