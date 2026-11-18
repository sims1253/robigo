Byte-exact inputs for the readLines and pushBack cases (robigo-rt has no
`writeBin` yet to make them): CR and CRLF line ends, an embedded NUL, a
UTF-8 BOM, an incomplete final line, an empty file.
