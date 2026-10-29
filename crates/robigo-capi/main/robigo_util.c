/* robigo: functions of GNU R 4.5.0's src/main the C API exports, copied
   from their files (memory.c's string buffers, util.c's acopy_string and
   R_strtod family, sort.c's sortVector), each marked with its origin. */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#include <Defn.h>
#include <Internal.h>
#include <R_ext/Print.h>
#include <ctype.h>
#include <float.h>
#include <errno.h>
#include "RBufferUtils.h"
#include <Rmath.h>
#include <R_ext/RS.h>
#include <R_ext/Itermacros.h>

/* R: memory.c (RBufferUtils) */
void *R_AllocStringBuffer(size_t blen, R_StringBuffer *buf)
{
    size_t blen1, bsize = buf->defaultSize;

    /* for backwards compatibility, this used to free the buffer */
    if(blen == (size_t)-1)
	error("R_AllocStringBuffer( (size_t)-1 ) is no longer allowed");

    if(blen * sizeof(char) < buf->bufsize) return buf->data;
    blen1 = blen = (blen + 1) * sizeof(char);
    blen = (blen / bsize) * bsize;
    if(blen < blen1) blen += bsize;

    /* Result may be accessed as `wchar_t *` and other types; malloc /
      realloc guarantee correct memory alignment for all object types */
    if(buf->data == NULL) {
	buf->data = (char *) malloc(blen);
	if(buf->data)
	    buf->data[0] = '\0';
    } else
	buf->data = (char *) realloc(buf->data, blen);
    buf->bufsize = blen;
    if(!buf->data) {
	buf->bufsize = 0;
	/* don't translate internal error message */
	error("could not allocate memory (%u %s) in C function 'R_AllocStringBuffer'",
	      (unsigned int) blen/1024/1024, "Mb");
    }
    return buf->data;
}

void R_FreeStringBuffer(R_StringBuffer *buf)
{
    if (buf->data != NULL) {
	free(buf->data);
	buf->bufsize = 0;
	buf->data = NULL;
    }
}

attribute_hidden void R_FreeStringBufferL(R_StringBuffer *buf)
{
    if (buf->bufsize > buf->defaultSize) {
	free(buf->data);
	buf->bufsize = 0;
	buf->data = NULL;
    }
}

/* R: util.c */
/* Return a copy of a string using memory from R_alloc.
   NB: caller has to manage R_alloc stack.  Used in platform.c
*/
char *acopy_string(const char *in)
{
    char *out;
    size_t len = strlen(in);
    if (len > 0) {
	out = (char *) R_alloc(1 + len, sizeof(char));
	strcpy(out, in);
    } else
	out = "";
    return out;
}


double R_strtod5(const char *str, char **endptr, char dec,
		 Rboolean NA, int exact)
{
    LDOUBLE ans = 0.0;
    int sign = 1;
    const char *p = str;

    /* optional whitespace */
    while (isspace(*p)) p++;

    if (NA && strncmp(p, "NA", 2) == 0) {
	ans = NA_REAL;
	p += 2;
	goto done;
    }

   /* optional sign */
    switch (*p) {
    case '-': sign = -1;
    case '+': p++;
    default: ;
    }

    if (strncasecmp(p, "NaN", 3) == 0) {
	ans = R_NaN;
	p += 3;
	goto done;
    /* C99 specifies this: must come first to avoid 'inf' match */
    } else if (strncasecmp(p, "infinity", 8) == 0) {
	ans = R_PosInf;
	p += 8;
	goto done;
    } else if (strncasecmp(p, "Inf", 3) == 0) {
	ans = R_PosInf;
	p += 3;
	goto done;
    }

    int n, expn = 0;
    if(strlen(p) > 2 && p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) { // Hexadecimal "0x....."
	/* Prior to 4.5.0 this did not allow forms such as 0x1.234
	   without an exponent.: C99 allow this and implicitly
	   appends "p0"".

	   Changed following PR#18805
	 */
	int exph = -1;

	/* This will overflow to Inf if appropriate */
	for(p += 2; p; p++) {
	    if('0' <= *p && *p <= '9') ans = 16*ans + (*p -'0');
	    else if('a' <= *p && *p <= 'f') ans = 16*ans + (*p -'a' + 10);
	    else if('A' <= *p && *p <= 'F') ans = 16*ans + (*p -'A' + 10);
	    else if(*p == dec) {exph = 0; continue;}
	    else break;
	    if (exph >= 0) exph += 4;
	}
#define strtod_EXACT_CLAUSE						\
	if(exact && ans > 0x1.fffffffffffffp52) {			\
	    if(exact == NA_LOGICAL)					\
		warning(_(						\
		"accuracy loss in conversion from \"%s\" to numeric"),	\
			str);						\
	    else {							\
		ans = NA_REAL;						\
		p = str; /* back out */					\
		goto done;						\
	    }								\
	}
	strtod_EXACT_CLAUSE;
	/* Binary exponent, if any */
	if (*p == 'p' || *p == 'P') {
	    int expsign = 1;
	    switch(*++p) {
	    case '-': expsign = -1;
	    case '+': p++;
	    default: ;
	    }
#define MAX_EXPONENT_PREFIX 9999
	    /* exponents beyond ca +1024/-1076 over/underflow 
	       Limit exponent from PR#16358.
	     */
	    int ndig = 0;
	    for (n = 0; *p >= '0' && *p <= '9'; p++, ndig++)
		n = (n < MAX_EXPONENT_PREFIX) ? n * 10 + (*p - '0') : n;
	    if (ndig == 0) {
		ans = NA_REAL;
		p = str; /* back out */
		goto done;
	    }
	    expn += expsign * n;
	}
	if (ans != 0.0) { /* PR#15976:  allow big exponents on 0 */
	    LDOUBLE fac = 1.0;
	    double p2 = 2.0;
	    if(exph > 0) {
		if (expn - exph < -122) {	/* PR#17199:  fac may overflow below if expn - exph is too small.
					       2^-122 is a bit bigger than 1E-37, so should be fine on all systems */
		    for (n = exph, fac = 1.0; n; n >>= 1, p2 *= p2)
			if (n & 1) fac *= p2;
		    ans /= fac;
		    p2 = 2.0;
		} else
		    expn -= exph;
	    }
	    if (expn < 0) {
		for (n = -expn, fac = 1.0; n; n >>= 1, p2 *= p2)
		    if (n & 1) fac *= p2;
		ans /= fac;
	    } else {
		for (n = expn, fac = 1.0; n; n >>= 1, p2 *= p2)
		    if (n & 1) fac *= p2;
		ans *= fac;
	    }
	}
	goto done;
    } // end {hexadecimal case}

    int ndigits = 0;
    for ( ; *p >= '0' && *p <= '9'; p++, ndigits++) ans = 10*ans + (*p - '0');
    if (*p == dec)
	for (p++; *p >= '0' && *p <= '9'; p++, ndigits++, expn--)
	    ans = 10*ans + (*p - '0');
    if (ndigits == 0) {
	ans = NA_REAL;
	p = str; /* back out */
	goto done;
    }
    strtod_EXACT_CLAUSE;

    if (*p == 'e' || *p == 'E') {
	int ndigits = 0;
	int expsign = 1;
	switch(*++p) {
	case '-': expsign = -1;
	case '+': p++;
	default: ;
	}
	/* The test for n is in response to PR#16358; which was
	   parsing 1e999999999999.
	   It's not right if the exponent is very large, but the
	   overflow or underflow below will handle it.
	   1e308 is already Inf, but negative exponents can go down to -323
	   before undeflowing to zero.  And people could do perverse things
	   like 0.00000001e312.
	*/
	// C17 §6.4.4.2 requires a non-empty 'digit sequence'
	for (n = 0; *p >= '0' && *p <= '9'; p++, ndigits++)
	    n = (n < MAX_EXPONENT_PREFIX) ? n * 10 + (*p - '0') : n;
	if (ndigits == 0) {
	    ans = NA_REAL;
	    p = str; /* back out */
	    goto done;
	}
	expn += expsign * n;
    }

    /* avoid unnecessary underflow for large negative exponents */
    if (expn + ndigits < -300) {
	for (n = 0; n < ndigits; n++) ans /= 10.0;
	expn += ndigits;
    }
    LDOUBLE p10 = 10., fac = 1.0;
    if (expn < -307) { /* use underflow, not overflow */
	for (n = -expn, fac = 1.0; n; n >>= 1, p10 *= p10)
	    if (n & 1) fac /= p10;
	ans *= fac;
    } else if (expn < 0) { /* positive powers are exact */
	for (n = -expn, fac = 1.0; n; n >>= 1, p10 *= p10)
	    if (n & 1) fac *= p10;
	ans /= fac;
    } else if (ans != 0.0) { /* PR#15976:  allow big exponents on 0, e.g. 0E4933 */
	for (n = expn, fac = 1.0; n; n >>= 1, p10 *= p10)
	    if (n & 1) fac *= p10;
	ans *= fac;
    }

    /* explicit overflow to infinity */
    if (ans > DBL_MAX) {
	if (endptr) *endptr = (char *) p;
	return (sign > 0) ? R_PosInf : R_NegInf;
    }

done:
    if (endptr) *endptr = (char *) p;
    return sign * (double) ans;
}


attribute_hidden
double R_strtod4(const char *str, char **endptr, char dec, Rboolean NA)
{
    return R_strtod5(str, endptr, dec, NA, FALSE);
}

double R_strtod(const char *str, char **endptr)
{
    return R_strtod5(str, endptr, '.', FALSE, FALSE);
}

double R_atof(const char *str)
{
    return R_strtod5(str, NULL, '.', FALSE, FALSE);
}


/* R: sort.c (the comparisons, isUnsorted, sortVector) */
static int icmp(int x, int y, bool nalast)
{
    if (x == NA_INTEGER && y == NA_INTEGER) return 0;
    if (x == NA_INTEGER)return nalast ? 1 : -1;
    if (y == NA_INTEGER)return nalast ? -1 : 1;
    if (x < y)		return -1;
    if (x > y)		return 1;
    return 0;
}

static int rcmp(double x, double y, bool nalast)
{
    int nax = ISNAN(x), nay = ISNAN(y);
    if (nax && nay)	return 0;
    if (nax)		return nalast ? 1 : -1;
    if (nay)		return nalast ? -1 : 1;
    if (x < y)		return -1;
    if (x > y)		return 1;
    return 0;
}

static int ccmp(Rcomplex x, Rcomplex y, bool nalast)
{
    int nax = ISNAN(x.r), nay = ISNAN(y.r);
				/* compare real parts */
    if (nax && nay)	return 0;
    if (nax)		return nalast ? 1 : -1;
    if (nay)		return nalast ? -1 : 1;
    if (x.r < y.r)	return -1;
    if (x.r > y.r)	return 1;
				/* compare complex parts */
    nax = ISNAN(x.i); nay = ISNAN(y.i);
    if (nax && nay)	return 0;
    if (nax)		return nalast ? 1 : -1;
    if (nay)		return nalast ? -1 : 1;
    if (x.i < y.i)	return -1;
    if (x.i > y.i)	return 1;

    return 0;		/* equal */
}

static int scmp(SEXP x, SEXP y, bool nalast)
{
    if (x == NA_STRING && y == NA_STRING) return 0;
    if (x == NA_STRING) return nalast ? 1 : -1;
    if (y == NA_STRING) return nalast ? -1 : 1;
    if (x == y) return 0;  /* same string in cache */
    return Scollate(x, y);
}

#define R_INT_MIN 1 + INT_MIN //INT_MIN is NA_INTEGER
// API: in Rinternals.h
Rboolean isUnsorted(SEXP x, Rboolean strictly)
{
    R_xlen_t n, i;
    int itmp = INT_MIN; /* this is NA_INTEGER, < all valid nonNA R integer
			   values */
    double dtmp = R_NegInf; /*  R_NegInf is min possible double	"value",
			        ***this is <= all nonNA R numeric values but
				NOT < all nonNA	values*** */
    if (!isVectorAtomic(x))
	error(_("only atomic vectors can be tested to be sorted"));
    n = XLENGTH(x);
    if(n >= 2)
	switch (TYPEOF(x)) {

	    /* NOTE: x must have no NAs {is.na(.) in R};
	       hence be faster than `rcmp()', `icmp()' for these two cases */

	    /* The only difference between strictly and not is '>' vs '>='
	       but we want the if() outside the loop */

	    /* x can be an ALTREP here provided that its sortedness is unknown, 
	       so we use ITERATE_BY_REGION to get the multiple INTEGER calls
	       outside of the tight loop and be ALTREP safe. */
	case LGLSXP:
	case INTSXP:
	    if(strictly) {
		ITERATE_BY_REGION(x, xptr, i, nbatch, int, INTEGER, {
			/* itmp initialized to INT_MIN which is < all
			   valid nonNA R int values so no need to
			   exclude first iteration

			   After first iteration itmp is value at end
			   of last batch */
			if(itmp >= xptr[0]) 
			    return TRUE;
			for(R_xlen_t k = 0; k < nbatch - 1; k++) {
			    if(xptr[k] >= xptr[k+1])
				return TRUE;
			}
			itmp = xptr[nbatch - 1];
		    });
	    } else {
		ITERATE_BY_REGION(x, xptr, i, nbatch, int, INTEGER, {
			if(itmp > xptr[0]) //deal with batch barriers
			    return TRUE;
			for(R_xlen_t k = 0; k < nbatch - 1; k++) {
			    if(xptr[k] > xptr[k+1])
				return TRUE;
			}
			itmp = xptr[nbatch - 1];
		    });
	    }
	    break;
	case REALSXP:
	    if(strictly) {
		ITERATE_BY_REGION(x, xptr, i, nbatch, double, REAL, {
			/* first element could be R_NegInf which is
			   initialization value of dtmp so don't do
			   the barrier check at i == 0 since its >=
			   here

			   After first iteration dtmp is value at end
			   of last batch */
			if(i > 0 && dtmp >= xptr[0]) //deal with batch barriers
			    return TRUE;
			for(R_xlen_t k = 0; k < nbatch - 1; k++) {
			    if(xptr[k] >= xptr[k+1])
				return TRUE;
			}
			dtmp = xptr[nbatch - 1];
		    });
	    } else {
		/* nonstrict, first element can never be < dtmp (== R_NegInf),
		   so no need to exclude first iteration in barrier check */
		ITERATE_BY_REGION(x, xptr, i, nbatch, double, REAL, {
			if(dtmp > xptr[0]) /* deal with batch barriers, dtmp
					      is end of last batch */
			    return TRUE;
			for(R_xlen_t k = 0; k < nbatch - 1; k++) {
			    if(xptr[k] > xptr[k+1])
				return TRUE;
			}
			dtmp = xptr[nbatch - 1];
		    });
	    }
	    break;
	case CPLXSXP:
	    if(strictly) {
		for(i = 0; i+1 < n ; i++)
		    if(ccmp(COMPLEX(x)[i], COMPLEX(x)[i+1], true) >= 0)
			return TRUE;
	    } else {
		for(i = 0; i+1 < n ; i++)
		    if(ccmp(COMPLEX(x)[i], COMPLEX(x)[i+1], true) > 0)
			return TRUE;
	    }
	    break;
	case STRSXP:
	    if(strictly) {
		for(i = 0; i+1 < n ; i++)
		    if(scmp(STRING_ELT(x, i ),
			    STRING_ELT(x,i+1), true) >= 0)
			return TRUE;
	    } else {
		for(i = 0; i+1 < n ; i++)
		    if(scmp(STRING_ELT(x, i ),
			    STRING_ELT(x,i+1), true) > 0)
			return TRUE;
	    }
	    break;
	case RAWSXP: // being compatible with raw_relop() in ./relop.c
	    if(strictly) {
		for(i = 0; i+1 < n ; i++)
		    if(RAW(x)[i] >= RAW(x)[i+1])
			return TRUE;

	    } else {
		for(i = 0; i+1 < n ; i++)
		    if(RAW(x)[i] > RAW(x)[i+1])
			return TRUE;
	    }
	    break;
	default:
	    UNIMPLEMENTED_TYPE("isUnsorted", x);
	}
    return FALSE;/* sorted */
} // isUnsorted()

/* c(1, 4^k +3*2^(k-1)+1) */
#ifdef LONG_VECTOR_SUPPORT
// This goes up to 2^38: extend eventually.
#define NI 20
static const R_xlen_t incs[NI + 1] = {
    274878693377L, 68719869953L, 17180065793L, 4295065601L,
    1073790977L, 268460033L, 67121153L, 16783361L, 4197377L, 1050113L,
    262913L, 65921L, 16577L, 4193L, 1073L, 281L, 77L, 23L, 8L, 1L, 0L
};
#else
#define NI 16
static const int incs[NI + 1] = {
    1073790977, 268460033, 67121153, 16783361, 4197377, 1050113,
    262913, 65921, 16577, 4193, 1073, 281, 77, 23, 8, 1, 0
};
#endif

#define sort2_body \
    for (h = incs[t]; t < NI; h = incs[++t]) \
	for (i = h; i < n; i++) { \
	    v = x[i]; \
	    j = i; \
	    while (j >= h && x[j - h] less v) { x[j] = x[j - h]; j -= h; } \
	    x[j] = v; \
	}

/* These are only called with n >= 2 */
static void R_isort2(int *x, R_xlen_t n, bool decreasing)
{
    int v;
    R_xlen_t i, j, h, t;

    if (n < 2) error("'n >= 2' is required");
    for (t = 0; incs[t] > n; t++);
    if(decreasing)
#define less <
	sort2_body
#undef less
    else
#define less >
	sort2_body
#undef less
}

static void R_rsort2(double *x, R_xlen_t n, bool decreasing)
{
    double v;
    R_xlen_t i, j, h, t;

    if (n < 2) error("'n >= 2' is required");
    for (t = 0; incs[t] > n; t++);
    if(decreasing)
#define less <
	sort2_body
#undef less
    else
#define less >
	sort2_body
#undef less
}

static void R_csort2(Rcomplex *x, R_xlen_t n, bool decreasing)
{
    Rcomplex v;
    R_xlen_t i, j, h, t;

    if (n < 2) error("'n >= 2' is required");
    for (t = 0; incs[t] > n; t++);
    for (h = incs[t]; t < NI; h = incs[++t])
	for (i = h; i < n; i++) {
	    v = x[i];
	    j = i;
	    if(decreasing)
		while (j >= h && (x[j - h].r < v.r ||
				  (x[j - h].r == v.r && x[j - h].i < v.i)))
		{ x[j] = x[j - h]; j -= h; }
	    else
		while (j >= h && (x[j - h].r > v.r ||
				  (x[j - h].r == v.r && x[j - h].i > v.i)))
		{ x[j] = x[j - h]; j -= h; }
	    x[j] = v;
	}
}

static void ssort2(SEXP *x, R_xlen_t n, bool decreasing)
{
    SEXP v;
    R_xlen_t i, j, h, t;

    if (n < 2) error("'n >= 2' is required");
    for (t = 0; incs[t] > n; t++);
    for (h = incs[t]; t < NI; h = incs[++t])
	for (i = h; i < n; i++) {
	    v = x[i];
	    j = i;
	    PROTECT(v);
	    if(decreasing)
		while (j >= h && scmp(x[j - h], v, true) < 0)
		{ x[j] = x[j - h]; j -= h; }
	    else
		while (j >= h && scmp(x[j - h], v, true) > 0)
		{ x[j] = x[j - h]; j -= h; }
	    x[j] = v;
	    UNPROTECT(1); /* v */
	}
}

/* The meat of sort.int() */
// Used in envir.c library/utils/src/io.c
void sortVector(SEXP s, bool decreasing)
{
    R_xlen_t n = XLENGTH(s);
    if (n >= 2 && (decreasing || isUnsorted(s, FALSE)))
	switch (TYPEOF(s)) {
	case LGLSXP:
	case INTSXP:
	    R_isort2(INTEGER(s), n, decreasing);
	    break;
	case REALSXP:
	    R_rsort2(REAL(s), n, decreasing);
	    break;
	case CPLXSXP:
	    R_csort2(COMPLEX(s), n, decreasing);
	    break;
	case STRSXP:
	    ssort2(STRING_PTR(s), n, decreasing);
	    break;
	default:
	    UNIMPLEMENTED_TYPE("sortVector", s);
	}
}
