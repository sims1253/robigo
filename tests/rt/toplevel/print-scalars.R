# Rscript's top level prints each visible value (PrintValueEnv): NULL and
# scalars of the four basic types, with print.default's formatting.
NULL
TRUE; FALSE; NA
1L; -7L; NA_integer_; 2147483647L
1; 0.5; 1/3; -2.5; 100000; 123456; 1234567; 1e-5; 0.0001; 1e15; 1e-300; 3.14159265358979
NA_real_; 0/0; 1/0; -1/0; 2^31; 0.1 + 0.2
"a"; ""; "tab\there"; "quote\"d"; "back\\slash"; "line\nbreak"; NA_character_
"été"; "日本"
