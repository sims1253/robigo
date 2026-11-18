/* robigo's Parse.h for GNU R's base-package natives: what GNU R 4.6.2's
 * src/include/Parse.h gives the natives robigo builds (the parser's status
 * type, R_ext/Parse.h's; the parser's globals are Defn.h's). robigo's own
 * parser is Rust, so GNU R's parser entry points are not here. */
#ifndef R_PARSE_H
#define R_PARSE_H

#include <Defn.h>
#include <R_ext/Parse.h>

/* 82 */
NORET void parseError(SEXP call, int linenum);

#endif /* not R_PARSE_H */
