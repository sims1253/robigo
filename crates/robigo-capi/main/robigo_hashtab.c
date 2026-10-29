/* robigo: the hash tables of GNU R 4.5.0's src/main/unique.c (the
   R_mkhashtab family, R_ext/... hashtab API) with the hashing they use,
   copied from that file: lines 20-429 (hash and equality functions) and
   2296-2644 (the hash tables). */


/* This is currently restricted to vectors of length < 2^30 */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#define R_USE_SIGNALS 1
#include <Defn.h>
#include <Internal.h>
#include <R_ext/Altrep.h>
#include <R_ext/Itermacros.h>

/* inline version of function R_NaN_is_R_NA defined in arithmetic.c */
/* may not be needed if LTO is enabled */
#define R_NaN_is_R_NA R_NaN_is_R_NA_inline
static R_INLINE int R_NaN_is_R_NA_inline(double x)
{
#ifdef WORDS_BIGENDIAN
    static const int lw = 1;
#else  /* !WORDS_BIGENDIAN */
    static const int lw = 0;
#endif /* WORDS_BIGENDIAN */
    union {
	double value;
	unsigned int word[2];
    } y;
    y.value = x;
    return y.word[lw] == 1954;
}

#define NIL -1
#define ARGUSED(x) LEVELS(x)
#define SET_ARGUSED(x,v) SETLEVELS(x,v)

/* interval at which to check interrupts */
#define NINTERRUPT 1000000

typedef size_t hlen;

/* Hash function and equality test for keys */
typedef struct _HashData HashData;

struct _HashData {
    int K;
    hlen M;
    R_xlen_t nmax;
#ifdef LONG_VECTOR_SUPPORT
    Rboolean isLong;
#endif
    hlen (*hash)(SEXP, R_xlen_t, HashData *);
    int (*equal)(SEXP, R_xlen_t, SEXP, R_xlen_t);
    SEXP HashTable;

    int nomatch;
    Rboolean useUTF8;
    Rboolean useCache;
    Rboolean useCloEnv;
    Rboolean extptrAsRef;
    Rboolean inHashtab;
};

#define HTDATA_INT(d) (INTEGER0((d)->HashTable))
#define HTDATA_DBL(d) (REAL0((d)->HashTable))


/*
   Integer keys are hashed via a random number generator
   based on Knuth's recommendations.  The high order K bits
   are used as the hash code.

   NB: lots of this code relies on M being a power of two and
   on silent integer overflow mod 2^32.

   <FIXME> Integer keys are wasteful for logical and raw vectors, but
   the tables are small in that case.  It would be much easier to
   implement long vectors, though.
*/

/*  Currently the hash table is implemented as a (signed) integer
    array.  So there are two 31-bit restrictions, the length of the
    array and the values.  The values are initially NIL (-1).  O-based
    indices are inserted by isDuplicated, and invalidated by setting
    to NA_INTEGER.
*/

static hlen scatter(unsigned int key, HashData *d)
{
    return 3141592653U * key >> (32 - d->K);
}

static hlen lhash(SEXP x, R_xlen_t indx, HashData *d)
{
    int xi = LOGICAL_ELT(x, indx);
    if (xi == NA_LOGICAL) return 2U;
    return (hlen) xi;
}

static R_INLINE hlen ihash(SEXP x, R_xlen_t indx, HashData *d)
{
    int xi = INTEGER_ELT(x, indx);
    if (xi == NA_INTEGER) return 0;
    return scatter((unsigned int) xi, d);
}

/* We use unions here because Solaris gcc -O2 has trouble with
   casting + incrementing pointers.  We use tests here, but R currently
   assumes int is 4 bytes and double is 8 bytes.
 */
union foo {
    double d;
    unsigned int u[2];
};

static R_INLINE hlen rhash(SEXP x, R_xlen_t indx, HashData *d)
{
    /* There is a problem with signed 0s under IEC60559 */
    double xi = REAL_ELT(x, indx);
    double tmp = (xi == 0.0) ? 0.0 : xi;
    /* need to use both 32-byte chunks or endianness is an issue */
    /* we want all NaNs except NA equal, and all NAs equal */
    if (R_IsNA(tmp)) tmp = NA_REAL;
    else if (R_IsNaN(tmp)) tmp = R_NaN;
#if 2*SIZEOF_INT == SIZEOF_DOUBLE
    {
	union foo tmpu;
	tmpu.d = tmp;
	return scatter(tmpu.u[0] + tmpu.u[1], d);
    }
#else
    return scatter(*((unsigned int *) (&tmp)), d);
#endif
}

static Rcomplex unify_complex_na(Rcomplex z) {
    Rcomplex ans;
    ans.r = (z.r == 0.0) ? 0.0 : z.r;
    ans.i = (z.i == 0.0) ? 0.0 : z.i;
    if (R_IsNA(ans.r) || R_IsNA(ans.i))
	ans.r = ans.i = NA_REAL;
    else if (R_IsNaN(ans.r) || R_IsNaN(ans.i))
	ans.r = ans.i = R_NaN;
    return ans;
}

static hlen chash(SEXP x, R_xlen_t indx, HashData *d)
{
    Rcomplex tmp = unify_complex_na(COMPLEX_ELT(x, indx));

#if 2*SIZEOF_INT == SIZEOF_DOUBLE
    {
	unsigned int u;
	union foo tmpu;
	tmpu.d = tmp.r;
	u = tmpu.u[0] ^ tmpu.u[1];
	tmpu.d = tmp.i;
	u ^= tmpu.u[0] ^ tmpu.u[1];
	return scatter(u, d);
    }
#else
	return scatter((*((unsigned int *)(&tmp.r)) ^
			(*((unsigned int *)(&tmp.i)))), d);
#endif
}

/* Pointer hashing as used here isn't entirely portable (we do it in
   several other places, sometimes in slightly different ways) but it
   could be made so by computing a unique value based on the
   allocation page and position in the page.

   Pointer hashes will not be valid if serialized and unserialized in
   another process.

   Hash values are int, For 64bit pointers, we do (upper ^ lower) */
static R_INLINE unsigned int PTRHASH(void *x)
{
    intptr_t z = (intptr_t) x;
    unsigned int z1 = (unsigned int)(z & 0xffffffff), z2 = 0;
#if SIZEOF_LONG == 8
    z2 = (unsigned int)(z/0x100000000L);
#endif
    return z1 ^ z2;
}

/* Hash CHARSXP by address. */
static R_INLINE hlen cshash(SEXP x, R_xlen_t indx, HashData *d)
{
    return scatter(PTRHASH(STRING_ELT(x, indx)), d);
}

static R_INLINE hlen shash(SEXP x, R_xlen_t indx, HashData *d)
{
    unsigned int k;
    const char *p;
    if (d->inHashtab) {
	SEXP xi = STRING_ELT(x, indx);
	int noTrans = (IS_BYTES(xi) || IS_ASCII(xi)) ? TRUE : FALSE;
	if (d->useCache && noTrans)
	    return scatter(PTRHASH(xi), d);
	const void *vmax = vmaxget();
	p = noTrans ? CHAR(xi) : translateCharUTF8(xi);
	k = 0;
	while (*p++)
	    /* multiplier was 8 but 11 isn't a power of 2 */
	    k = 11 * k + (unsigned int) *p;
	vmaxset(vmax); /* discard any memory used by translateChar */
	return scatter(k, d);
    }
    if(!d->useUTF8 && d->useCache) return cshash(x, indx, d);
    const void *vmax = vmaxget();
    /* Not having d->useCache really should not happen anymore. */
    p = translateCharUTF8(STRING_ELT(x, indx));
    k = 0;
    while (*p++)
	k = 11 * k + (unsigned int) *p; /* was 8 but 11 isn't a power of 2 */
    vmaxset(vmax); /* discard any memory used by translateChar */
    return scatter(k, d);
}

static int lequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    return (LOGICAL_ELT(x, i) == LOGICAL_ELT(y, j));
}


static R_INLINE int iequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    return (INTEGER_ELT(x, i) == INTEGER_ELT(y, j));
}

/* BDR 2002-1-17  We don't want NA and other NaNs to be equal */
static R_INLINE int requal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    double xi = REAL_ELT(x, i);
    double yj = REAL_ELT(y, j);
    if (!ISNAN(xi) && !ISNAN(yj))
	return (xi == yj);
    else if (R_IsNA(xi) && R_IsNA(yj)) return 1;
    else if (R_IsNaN(xi) && R_IsNaN(yj)) return 1;
    else return 0;
}

/* This is differentiating {NA,1}, {NA,0}, {NA, NaN}, {NA, NA},
 * but R's print() and format()  render all as "NA" */
static int cplx_eq(Rcomplex x, Rcomplex y)
{
    if (!ISNAN(x.r) && !ISNAN(x.i) && !ISNAN(y.r) && !ISNAN(y.i))
	return x.r == y.r && x.i == y.i;
    else if (R_IsNA(x.r) || R_IsNA(x.i)) // x is NA
	return (R_IsNA(y.r) || R_IsNA(y.i)) ? 1 : 0;
    else if (R_IsNA(y.r) || R_IsNA(y.i)) // y is NA but x is not
	return 0;
    // else : none is NA but there's at least one NaN;  hence  ISNAN(.) == R_IsNaN(.)
    return
	(((ISNAN(x.r) && ISNAN(y.r)) || (!ISNAN(x.r) && !ISNAN(y.r) && x.r == y.r)) && // Re
	 ((ISNAN(x.i) && ISNAN(y.i)) || (!ISNAN(x.i) && !ISNAN(y.i) && x.i == y.i))    // Im
	    ) ? 1 : 0;
}

static int cequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    return cplx_eq(COMPLEX_ELT(x, i), COMPLEX_ELT(y, j));
}

static R_INLINE int sequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    SEXP xi = STRING_ELT(x, i);
    SEXP yj = STRING_ELT(y, j);
    /* Two strings which have the same address must be the same,
       so avoid looking at the contents */
    if (xi == yj) return 1;
    /* Then if either is NA the other cannot be */
    /* Once all CHARSXPs are cached, Seql will handle this */
    if (xi == NA_STRING || yj == NA_STRING)
	return 0;
    /* another pre-test to avoid the call to Seql */
    if (IS_CACHED(xi) && IS_CACHED(yj) && ENC_KNOWN(xi) == ENC_KNOWN(yj))
	return 0;
    return Seql(xi, yj);
}

static hlen rawhash(SEXP x, R_xlen_t indx, HashData *d)
{
    return (hlen) RAW_ELT(x, indx);
}

static int rawequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    return (RAW_ELT(x, i) == RAW_ELT(y, j));
}

static hlen vhash_one(SEXP _this, HashData *d);
static hlen vhash(SEXP x, R_xlen_t indx, HashData *d)
{
    SEXP _this = VECTOR_ELT(x, indx);
    return vhash_one(_this, d);
}

static hlen vhash_one(SEXP _this, HashData *d)
{
    /* Handle environments by pointer hashing. Previously,
       environments were hashed based only on length, which is not
       very effective and could be expensive to compute. */
    if (TYPEOF(_this) == ENVSXP)
	return scatter(PTRHASH(_this), d);

    int i;
    unsigned int key = OBJECT(_this) + 2*TYPEOF(_this) + 100U*(unsigned int) length(_this);
    /* maybe we should also look at attributes, but that slows us down */
    switch (TYPEOF(_this)) {
    case LGLSXP:
	/* This is not too clever: pack into 32-bits and then scatter? */
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= lhash(_this, i, d);
	    key *= 97;
	}
	break;
    case INTSXP:
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= ihash(_this, i, d);
	    key *= 97;
	}
	break;
    case REALSXP:
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= rhash(_this, i, d);
	    key *= 97;
	}
	break;
    case CPLXSXP:
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= chash(_this, i, d);
	    key *= 97;
	}
	break;
    case STRSXP:
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= shash(_this, i, d);
	    key *= 97;
	}
	break;
    case RAWSXP:
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= scatter((unsigned int)rawhash(_this, i, d), d);
	    key *= 97;
	}
	break;
    case EXPRSXP:
    case VECSXP:
	R_CheckStack();
	for(i = 0; i < LENGTH(_this); i++) {
	    key ^= vhash(_this, i, d);
	    key *= 97;
	}
	break;
    case LANGSXP:
    case LISTSXP:
	R_CheckStack();
	/* all attributes are ignored */
	/* might be good to consider environments on formulas */
	for (SEXP next = _this; next != R_NilValue; next = CDR(next)) {
	    key ^= vhash_one(CAR(next), d);
	    key *= 97;
	}
	break;
    case CLOSXP:
	/* all attributes are ignored */
	key ^= vhash_one(BODY_EXPR(_this), d);
	key *= 97;
	if (d->useCloEnv) {
	    key ^= vhash_one(CLOENV(_this), d);
	    key *= 97;
	}
	break;
    case SYMSXP:
	key *= PTRHASH(_this);
	key *= 97;
	break;
    case CHARSXP:
	if(!d->useUTF8 && d->useCache) key *= PTRHASH(_this);
	/**** otherwise, do nothing for now */
	/* this should only happen in C-leve hash tables */
	/* eventually this should do what shash does */
	key *= 97;
	break;
    case EXTPTRSXP:
	if (d->extptrAsRef)
	    key ^= PTRHASH(_this);
	else
	    /* identical() considers only the EXTPTR_PTR values ... */
	    key ^= PTRHASH(EXTPTR_PTR(_this));
	key *= 97;
	break;
    default:
	break;
    }
    return scatter(key, d);
}

static int vequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    if (i < 0 || j < 0) return 0;
    return R_compute_identical(VECTOR_ELT(x, i), VECTOR_ELT(y, j), 0);
}

/***
 *** Experimental Hath Table Implementation
 ***
 *** The interface is based loosely on the design of the Common Lisp
 *** hash table support. Two equality tests are supported: identical()
 *** and pointer equality.
 ***/

/**
 ** C Level Interface
 **/

/*
 * Low Level Functions
 */

static int hash_identical(SEXP x, int K, Rboolean useCloEnv)
{
    /* using 31 seems to work reasonably */
    if (K == 0 || K > 31) K = 31;

    HashData d = { .K = K, .useUTF8 = FALSE, .useCache = TRUE };
    d.useCloEnv = useCloEnv;
    d.extptrAsRef = TRUE;
    d.inHashtab = TRUE;

    int val = (int) vhash_one(x, &d);
    if (val == NA_INTEGER) val = 0;
    if (val < 0) val = -val;
    return val;
}

static int hash_address(SEXP x, int K)
{
    if (K == 0 || K > 31) K = 31;

    HashData d = { .K = K };

    int val = (int) scatter(PTRHASH(x), &d);
    if (val == NA_INTEGER) val = 0;
    if (val < 0) val = -val;
    return val;
}

/* allow for compiling with NAMED */
static R_INLINE SEXP INC_NMD(SEXP x) {
    INCREMENT_NAMED(x);
    return x;
}

#define HT_SEXP(h) (h).cell
#define HT_META_SIZE 3
#define HT_META(h) R_ExternalPtrTag(HT_SEXP(h))

#define HT_TABLE(h) R_ExternalPtrProtected(HT_SEXP(h))
#define SET_HT_TABLE(h, table) R_SetExternalPtrProtected(HT_SEXP(h), table)
#define HT_TABLE_SIZE(h) LENGTH(HT_TABLE(h))

#define HT_COUNT(h) (INTEGER(HT_META(h))[0])
#define HT_TYPE(h) (INTEGER(HT_META(h))[1])
#define HT_TABLE_K(h) (INTEGER(HT_META(h))[2])

#define HT_IS_VALID(h) (R_ExternalPtrAddr(HT_SEXP(h)) != NULL)
#define HT_VALIDATE(h) R_SetExternalPtrAddr(HT_SEXP(h), HT_SEXP(h))

static R_INLINE int HT_HASH(R_hashtab_type h, SEXP key)
{
    switch(HT_TYPE(h)) {
    case HT_TYPE_IDENTICAL:
	return hash_identical(key, HT_TABLE_K(h), TRUE);
    case HT_TYPE_ADDRESS:
	return hash_address(key, HT_TABLE_K(h));
    default:
	error("bad hash table type");
    }
}

static R_INLINE int HT_EQUAL(R_hashtab_type h, SEXP x, SEXP y)
{
    switch(HT_TYPE(h)) {
    case HT_TYPE_IDENTICAL:
	{
	    /* IDENT_USE_CLOENV corresponds to the default in identical().
	       IDENT_EXTPTR_AS_REF ensures that EXTPTRSXP objects in
	       keys unserialize sensibly. */
	    int flags = IDENT_USE_CLOENV | IDENT_EXTPTR_AS_REF;
	    return R_compute_identical(x, y, flags);
	}
    case HT_TYPE_ADDRESS:
	return x == y;
    default:
	error("bad hash table type");
    }
}

static void rehash(R_hashtab_type h, int resize)
{
    /* If the meta-data structure is changed then a saved hash table
       will be invalid. For now, just check and reject incompatible
       oned. For future changes would be better to try to
       rewrite/repair the meta data. */
    if (TYPEOF(HT_META(h)) != INTSXP || LENGTH(HT_META(h)) != HT_META_SIZE)
	error("invalid hash table meta data");

    SEXP old_table = PROTECT(HT_TABLE(h));
    int old_size = LENGTH(old_table);

    R_xlen_t new_size = resize ? 2 * old_size : old_size;
    if (new_size > INT_MAX)
	error("hash size would exceed limit");

    HT_COUNT(h) = 0;
    HT_VALIDATE(h);
    
    SET_HT_TABLE(h, allocVector(VECSXP, new_size));
    if (resize) HT_TABLE_K(h)++;

    for (int i = 0; i < old_size; i++)
	for (SEXP cell = VECTOR_ELT(old_table, i);
	     cell != R_NilValue;
	     cell = CDR(cell))
	    R_sethash(h, TAG(cell), CAR(cell));

    UNPROTECT(1); /* old_table */
}

static SEXP getcell(R_hashtab_type h, SEXP key, int *pidx)
{
    SEXP table = HT_TABLE(h);

    if (! HT_IS_VALID(h))
	rehash(h, FALSE);

    int idx = HT_HASH(h, key);
    *pidx = idx;

    SEXP chain = VECTOR_ELT(table, idx);
    while (chain != R_NilValue) {
	if (HT_EQUAL(h, TAG(chain), key))
	    return chain;
	chain = CDR(chain);
    }
    return R_NilValue;
}    


/*
 * Higer Level Public Functions
 */

/* initial size = 2 ^ HT_INIT_K */
#define HT_INIT_K 3

R_hashtab_type R_mkhashtab(int type, int K)
{
    if (K < 3 || K > 30) K = 3;
    int size = 1;
    for (int i = 0; i < K; i++) size *= 2;

    switch(type) {
    case HT_TYPE_IDENTICAL:
    case HT_TYPE_ADDRESS: break;
    default: error("bad hash table type");
    }
    SEXP table = PROTECT(allocVector(VECSXP, size));
    SEXP meta = PROTECT(allocVector(INTSXP, HT_META_SIZE));
    R_hashtab_type val = { .cell = R_MakeExternalPtr(NULL, meta, table) };
    HT_VALIDATE(val);
    HT_COUNT(val) = 0;
    HT_TYPE(val) = type;
    HT_TABLE_K(val) = K;
    UNPROTECT(2); /* table, meta */
    return val;
}

SEXP R_gethash(R_hashtab_type h, SEXP key, SEXP nomatch)
{
    int idx;
    PROTECT(HT_SEXP(h));
    PROTECT(key);
    PROTECT(nomatch);
    SEXP cell = getcell(h, key, &idx);
    UNPROTECT(3); /* h, key, nomatch */
    if (cell == R_NilValue)
	return nomatch;
    else
	return CAR(cell);
}

SEXP R_sethash(R_hashtab_type h, SEXP key, SEXP value)
{
    int idx;
    PROTECT(HT_SEXP(h));
    PROTECT(key);
    PROTECT(value);
    SEXP cell = getcell(h, key, &idx);
    if (cell == R_NilValue) {
	int new_count = HT_COUNT(h) + 1;
	if (new_count > 0.5 * HT_TABLE_SIZE(h)) {
	    /* rehash() will fail if new_count would be too large */
	    rehash(h, TRUE);
	    /* recalculate the hash bin, cell will still be R_NilValue */
	    (void) getcell(h, key, &idx);
	}
	SEXP table = HT_TABLE(h);
	SEXP chain = CONS(INC_NMD(value), VECTOR_ELT(table, idx));
	SET_TAG(chain, INC_NMD(key));
	SET_VECTOR_ELT(table, idx, chain);
	HT_COUNT(h) = new_count;
    }
    else {
	SETCAR(cell, value);
	INCREMENT_NAMED(value);
    }
    UNPROTECT(3); /* h, key, value */
    return value;
}

int R_remhash(R_hashtab_type h, SEXP key)
{
    int idx;
    PROTECT(HT_SEXP(h));
    PROTECT(key);
    SEXP cell = getcell(h, key, &idx);
    UNPROTECT(2); /* h, key */

    if (cell == R_NilValue)
	return FALSE;
    else {
	SEXP table = HT_TABLE(h);
	if (cell == VECTOR_ELT(table, idx))
	    SET_VECTOR_ELT(table, idx, CDR(cell));
	else {
	    SEXP prev = VECTOR_ELT(table, idx);
	    while (CDR(prev) !=  cell)
		prev = CDR(prev);
	    SETCDR(prev, CDR(cell));
	}
	HT_COUNT(h)--;
	SETCAR(cell, R_NilValue);  // drop REFCNT on old value
	SET_TAG(cell, R_NilValue); // drop REFCNT on old key
	return TRUE;
    }
}

int R_numhash(R_hashtab_type h) { return HT_COUNT(h); }
int R_typhash(R_hashtab_type h) { return HT_TYPE(h); }

static R_INLINE void defvar(SEXP sym, SEXP val, SEXP env)
{
    defineVar(sym, INC_NMD(val), env);
}

SEXP R_maphash(R_hashtab_type h, SEXP FUN)
{
    PROTECT(HT_SEXP(h));
    PROTECT(FUN);
    SEXP FUN_sym = install("FUN");
    SEXP key_sym = install("key");
    SEXP val_sym = install("value");

    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    SEXP call = PROTECT(lang3(FUN_sym, key_sym, val_sym));
    defvar(FUN_sym, FUN, env);
    
    SEXP table = PROTECT(HT_TABLE(h)); // PROTECT in case FUN causes a rehash
    int size = LENGTH(table);
    for (int i = 0; i < size; i++) {
	SEXP cell = VECTOR_ELT(table, i);
	while (cell != R_NilValue) {
	    SEXP next = PROTECT(CDR(cell));
	    defvar(key_sym, TAG(cell), env); // key is PROTECTed by env
	    defvar(val_sym, CAR(cell), env); // val is PROTECTed by env
	    eval(call, env);
	    cell = next;
	    UNPROTECT(1); /* next */
	}
    }
    UNPROTECT(5); /* h, FUN, env, call, table */
    return R_NilValue;
}

void R_maphashC(R_hashtab_type h, void (*FUN)(SEXP, SEXP, void *), void *data)
{
    PROTECT(HT_SEXP(h));
    SEXP table = PROTECT(HT_TABLE(h)); // PROTECT in case FUN causes a rehash
    int size = LENGTH(table);
    for (int i = 0; i < size; i++) {
	SEXP cell = VECTOR_ELT(table, i);
	while (cell != R_NilValue) {
	    SEXP next = PROTECT(CDR(cell));
	    SEXP key = PROTECT(TAG(cell));
	    SEXP val = PROTECT(CAR(cell));
	    FUN(key, val, data);
	    cell = next;
	    UNPROTECT(3); /* next, key, val */
	}
    }
    UNPROTECT(2); /* h, table */
}

void R_clrhash(R_hashtab_type h)
{
    SEXP table = HT_TABLE(h);
    int size = LENGTH(table);
    for (int i = 0; i < size; i++) {
	for (SEXP cell = VECTOR_ELT(table, i);
	     cell != R_NilValue;
	     cell = CDR(cell)) {
	    SETCAR(cell, R_NilValue);  // drop REFCNT on old value
	    SET_TAG(cell, R_NilValue); // drop REFCNT on old key
	}
	SET_VECTOR_ELT(table, i, R_NilValue);
    }
    HT_COUNT(h) = 0;
    /* could also drop table size back down to HT_INIT_SIZE */
}


/**
 ** R Level Interface Support
 **/

R_hashtab_type R_asHashtable(SEXP h)
{
    if (TYPEOF(h) != VECSXP || LENGTH(h) != 1 || ! inherits(h, "hashtab"))
	error("not a proper hash table object");
    SEXP p = VECTOR_ELT(h, 0);
    if (TYPEOF(p) != EXTPTRSXP)
	error("hash table object is corrupted");
    R_hashtab_type val = { .cell = p };
    return val;
}

SEXP R_HashtabSEXP(R_hashtab_type  h)
{
    return HT_SEXP(h);
}

int R_isHashtable(SEXP h)
{
    if (TYPEOF(h) != VECSXP ||
	LENGTH(h) != 1 ||
	! inherits(h, "hashtab") ||
	TYPEOF(VECTOR_ELT(h, 0)) != EXTPTRSXP)
	return FALSE;
    else
	return TRUE;
}

/* R: unique.c lines 440-561 (MKsetup, HashTableSetup, isDuplicated) and
   2216-2249 (csequal, HashTableSetup1, Rf_csduplicated) */
static void MKsetup(R_xlen_t n, HashData *d, R_xlen_t nmax)
{
#ifdef LONG_VECTOR_SUPPORT
    /* M = 2^32 is safe, hence n <= 2^31 -1 */
    if(n < 0) /* protect against overflow to -ve */
	error(_("length %lld is too large for hashing"), (long long)n);
#else
    if(n < 0 || n >= 1073741824) /* protect against overflow to -ve */
	error(_("length %d is too large for hashing"), n);
#endif

    if (nmax != NA_INTEGER && nmax != 1) n = nmax;
    size_t n2 = 2U * (size_t) n;
    d->M = 2;
    d->K = 1;
    while (d->M < n2) {
	d->M *= 2;
	d->K++;
    }
    d->nmax = n;
}

#define IMAX 4294967296L
static void HashTableSetup(SEXP x, HashData *d, R_xlen_t nmax)
{
    d->useUTF8 = FALSE;
    d->useCache = TRUE;
    switch (TYPEOF(x)) {
    case LGLSXP:
	d->hash = lhash;
	d->equal = lequal;
	d->nmax = d->M = 4;
	d->K = 2; /* unused */
	break;
    case INTSXP:
    {
	d->hash = ihash;
	d->equal = iequal;
#ifdef LONG_VECTOR_SUPPORT
	R_xlen_t nn = XLENGTH(x);
	if (nn > IMAX) nn = IMAX;
	MKsetup(nn, d, nmax);
#else
	MKsetup(LENGTH(x), d, nmax);
#endif
    }
	break;
    case REALSXP:
	d->hash = rhash;
	d->equal = requal;
	MKsetup(XLENGTH(x), d, nmax);
	break;
    case CPLXSXP:
	d->hash = chash;
	d->equal = cequal;
	MKsetup(XLENGTH(x), d, nmax);
	break;
    case STRSXP:
	d->hash = shash;
	d->equal = sequal;
	MKsetup(XLENGTH(x), d, nmax);
	break;
    case RAWSXP:
	d->hash = rawhash;
	d->equal = rawequal;
	d->nmax = d->M = 256;
	d->K = 8; /* unused */
	break;
    case EXPRSXP:
    case VECSXP:
	d->hash = vhash;
	d->equal = vequal;
	MKsetup(XLENGTH(x), d, nmax);
	break;
    default:
	UNIMPLEMENTED_TYPE("HashTableSetup", x);
    }
#ifdef LONG_VECTOR_SUPPORT
    d->isLong = IS_LONG_VEC(x);
    if (d->isLong) {
	d->HashTable = allocVector(REALSXP, (R_xlen_t) d->M);
	for (R_xlen_t i = 0; i < d->M; i++) HTDATA_DBL(d)[i] = NIL;
    } else
#endif
    {
	d->HashTable = allocVector(INTSXP, (R_xlen_t) d->M);
	for (R_xlen_t i = 0; i < d->M; i++) HTDATA_INT(d)[i] = NIL;
    }
}

/* Open address hashing */
/* Collision resolution is by linear probing */
/* The table is guaranteed large so this is sufficient */

static int isDuplicated(SEXP x, R_xlen_t indx, HashData *d)
{
#ifdef LONG_VECTOR_SUPPORT
    if (d->isLong) {
	double *h = HTDATA_DBL(d);
	hlen i = d->hash(x, indx, d);
	while (h[i] != NIL) {
	    if (d->equal(x, (R_xlen_t) h[i], x, indx))
		return h[i] >= 0 ? 1 : 0;
	    i = (i + 1) % d->M;
	}
	if (d->nmax-- < 0) error("hash table is full");
	h[i] = (double) indx;
    } else
#endif
    {
	int *h = HTDATA_INT(d);
	hlen i = d->hash(x, indx, d);
	while (h[i] != NIL) {
	    if (d->equal(x, h[i], x, indx))
		return h[i] >= 0 ? 1 : 0;
	    i = (i + 1) % d->M;
	}
	if (d->nmax-- < 0) error("hash table is full");
	h[i] = (int) indx;
    }
    return 0;
}
static int csequal(SEXP x, R_xlen_t i, SEXP y, R_xlen_t j)
{
    return STRING_ELT(x, i) == STRING_ELT(y, j);
}

static void HashTableSetup1(SEXP x, HashData *d)
{
    d->hash = cshash;
    d->equal = csequal;
#ifdef LONG_VECTOR_SUPPORT
    d->isLong = FALSE;
#endif
    MKsetup(XLENGTH(x), d, NA_INTEGER);
    d->HashTable = allocVector(INTSXP, (R_xlen_t) d->M);
    for (R_xlen_t i = 0; i < d->M; i++) HTDATA_INT(d)[i] = NIL;
}

/* used in utils */
SEXP Rf_csduplicated(SEXP x)
{
    if(TYPEOF(x) != STRSXP)
	error("C function 'csduplicated' not called on a STRSXP");
    R_xlen_t n = XLENGTH(x);
    HashData data = { 0 };
    HashTableSetup1(x, &data);
    PROTECT(data.HashTable);
    SEXP ans = PROTECT(allocVector(LGLSXP, n));
    int *v = LOGICAL(ans);

    for (R_xlen_t i = 0; i < n; i++) v[i] = isDuplicated(x, i, &data);

    UNPROTECT(2);
    return ans;
}
