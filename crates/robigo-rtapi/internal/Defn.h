/* robigo's Defn.h for GNU R's base-package natives (docs/g3.md §10, the A5
 * note's decision H): GNU R 4.6.2's src/include/Defn.h is the spec, and each
 * declaration here has GNU R's name, type and size (its line in a comment).
 * What the natives robigo builds use is here; robigo-rtcore's Defn.h,
 * included next, holds the contexts and the sxpinfo masks, which the
 * runtime shares. There is no USE_RINTERNALS: the accessors are the API's
 * functions, and GNU R's per-session globals are per isolate, macros over
 * robigo_cg (robigo_capi.h), as robigo's Rinternals.h makes them. */
#ifndef ROBIGO_NATIVES_DEFN_H_
#define ROBIGO_NATIVES_DEFN_H_

#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <R.h>
#include <Rinternals.h>
#include_next <Defn.h>

#define MAXELTSIZE 8192                        /* 85 */
#define FILESEP "/"                            /* 993 (Unix) */
#define streql(s, t) (!strcmp((s), (t)))       /* 1498 */
#define R_EOF -1                               /* 1531 */

/* the symbols (1553-1554) */
#define R_SrcfileSymbol (robigo_cg->R_SrcfileSymbol)
#define R_SrcrefSymbol (robigo_cg->R_SrcrefSymbol)

/* the parser's globals (1646-1655) */
#define PARSE_ERROR_SIZE 256
#define PARSE_CONTEXT_SIZE 256
/* (before the names become macros) */
_Static_assert(sizeof(((struct robigo_cglobals *) 0)->R_ParseErrorMsg) == PARSE_ERROR_SIZE, "R_ParseErrorMsg");
_Static_assert(sizeof(((struct robigo_cglobals *) 0)->R_ParseContext) == PARSE_CONTEXT_SIZE, "R_ParseContext");
#define R_ParseError (robigo_cg->R_ParseError)
#define R_ParseErrorMsg (robigo_cg->R_ParseErrorMsg)
#define R_ParseContext (robigo_cg->R_ParseContext)
#define R_ParseContextLast (robigo_cg->R_ParseContextLast)
#define R_ParseContextLine (robigo_cg->R_ParseContextLine)

/* the locale's (1680, 1682) */
#define mbcslocale (robigo_cg->mbcslocale)
#define R_MB_CUR_MAX (robigo_cg->R_MB_CUR_MAX)

/* contexts (1864, 1895, 2310, 2312) */
#define begincontext Rf_begincontext
#define endcontext Rf_endcontext
void begincontext(RCNTXT*, int, SEXP, SEXP, SEXP, SEXP, SEXP);
void endcontext(RCNTXT*);

/* files (2044, 2496) and the dirent wrappers (2116-2124) */
bool R_FileExists(const char *);
FILE *RC_fopen(const SEXP fn, const char *mode, const Rboolean expand);
struct R_dirent {
    char *d_name; /* null-terminated filename */
};
typedef struct R_DIR_INTERNAL R_DIR;
R_DIR *R_opendir(const char *name);
struct R_dirent *R_readdir(R_DIR *rdir);
int R_closedir(R_DIR *rdir);

/* environments (2062, 2270) */
SEXP Rf_findVar(SEXP, SEXP);
SEXP R_NewHashedEnv(SEXP, int);

/* multibyte strings (1949, 2444-2456) */
#define Mbrtowc Rf_mbrtowc
#define mbs_init(x) memset(x, 0, sizeof(mbstate_t))
SEXP Rf_installTrChar(SEXP);
size_t Mbrtowc(wchar_t *wc, const char *s, size_t n, mbstate_t *ps);
char *Rf_strchr(char *s, int c);
const char *Rf_strchr_const(const char *s, int c);

#endif /* ROBIGO_NATIVES_DEFN_H_ */
