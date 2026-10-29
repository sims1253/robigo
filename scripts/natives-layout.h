/* robigo: GNU R 4.5.0's object layout sizes (Defn.h's USE_RINTERNALS
   section, 64-bit), for base-package code that asks for USE_RINTERNALS
   only to measure objects (utils' size.c: object.size reports GNU R's
   sizes). scripts/build-natives.sh includes this after Defn.h, which is
   compiled without USE_RINTERNALS (robigo's objects are reached through
   the accessor functions). */

/* SEXPREC: sxpinfo, attrib, gengc_next_node, gengc_prev_node, 3 words */
struct SEXPREC { void *w[7]; };
typedef struct SEXPREC SEXPREC;
/* VECTOR_SEXPREC: the 4-word header, length, truelength */
typedef union { void *w[6]; double align; } SEXPREC_ALIGN;

/* Vector Heap Structure */
struct VECREC {
	union {
		SEXP		backpointer;
		double		align;
	} u;
};
typedef struct VECREC VECREC;

/* Vector Heap Macros */
#define BYTE2VEC(n)	(((n)>0)?(((n)-1)/sizeof(VECREC)+1):0)
#define INT2VEC(n)	(((n)>0)?(((n)*sizeof(int)-1)/sizeof(VECREC)+1):0)
#define FLOAT2VEC(n)	(((n)>0)?(((n)*sizeof(double)-1)/sizeof(VECREC)+1):0)
#define COMPLEX2VEC(n)	(((n)>0)?(((n)*sizeof(Rcomplex)-1)/sizeof(VECREC)+1):0)
#define PTR2VEC(n)	(((n)>0)?(((n)*sizeof(SEXP)-1)/sizeof(VECREC)+1):0)
