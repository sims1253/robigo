/* robigo: the parts of GNU R 4.5.0's print.c, printutils.c and options.c
   that format.c, the graphics engine and the graphics packages use,
   copied from those files (the R print parameters, the Encode*
   functions, and the globals options() sets). */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif

#include <Defn.h>
#include <Print.h>
#include <rlocale.h>
#include <wchar.h>

/* R: print.c */
R_PrintData R_print;

/* R: options.c: the globals options(OutDec=, max.contour.segments=) set;
   robigo's options() calls the setters below */
char *OutDec = ".";
unsigned int max_contour_segments = 25000;

void robigo_set_OutDec(const char *dec)
{
    static char sdec[11];
    strncpy(sdec, dec, 10);
    sdec[10] = '\0';
    OutDec = sdec;
}

void robigo_set_max_contour_segments(int k)
{
    max_contour_segments = k;
}

/* R: options.c:GetOptionCutoff */
int GetOptionCutoff(void)
{
    int w;
    w = asInteger(GetOption1(install("deparse.cutoff")));
    if (w == NA_INTEGER || w <= 0) {
	warning(_("invalid 'deparse.cutoff', used 60"));
	w = 60;
    }
    return w;
}

/* R: print.c:PrintInit */
attribute_hidden void PrintInit(R_PrintData *data, SEXP env)
{
    data->na_string = NA_STRING;
    data->na_string_noquote = mkChar("<NA>");
    data->na_width = Rstrlen(data->na_string, 0);
    data->na_width_noquote = Rstrlen(data->na_string_noquote, 0);
    data->quote = 1;
    data->right = Rprt_adj_left;
    data->digits = GetOptionDigits();
    data->scipen = asInteger(GetOption1(install("scipen")));
    if (data->scipen == NA_INTEGER) data->scipen = 0;
    data->max = asInteger(GetOption1(install("max.print")));
    if (data->max == NA_INTEGER || data->max < 0) data->max = 99999;
    else if(data->max == INT_MAX) data->max--; // so we can add
    data->gap = 1;
    data->width = GetOptionWidth();
    data->useSource = USESOURCE;
    data->cutoff = GetOptionCutoff();
    data->env = env;
    data->callArgs = R_NilValue;
}

/* R: print.c:PrintDefaults */
void PrintDefaults(void)
{
    PrintInit(&R_print, R_GlobalEnv);
}

/* R: printutils.c */
#define NB 1000
#define min(a, b) (((a)<(b))?(a):(b))

/* printutils.c: Rstrwid(x, strlen(x), CE_NATIVE, 0), for the decimal
   mark (printable characters: the sum of their widths) */
static int strwidth(const char *x)
{
    wchar_t wc[NB];
    size_t n = mbstowcs(wc, x, NB - 1);
    if (n == (size_t) -1) return (int) strlen(x);
    wc[n] = 0;
    return Ri18n_wcswidth(wc, n);
}

const char *EncodeLogical(int x, int w)
{
    static char buff[NB];
    if(x == NA_LOGICAL) snprintf(buff, NB, "%*s", min(w, (NB-1)), CHAR(R_print.na_string));
    else if(x) snprintf(buff, NB, "%*s", min(w, (NB-1)), "TRUE");
    else snprintf(buff, NB, "%*s", min(w, (NB-1)), "FALSE");
    buff[NB-1] = '\0';
    return buff;
}

const char *EncodeInteger(int x, int w)
{
    static char buff[NB];
    if(x == NA_INTEGER) snprintf(buff, NB, "%*s", min(w, (NB-1)), CHAR(R_print.na_string));
    else snprintf(buff, NB, "%*d", min(w, (NB-1)), x);
    buff[NB-1] = '\0';
    return buff;
}

const char *EncodeReal0(double x, int w, int d, int e, const char *dec)
{
    static char buff[NB], buff2[2*NB];
    char fmt[20], *out = buff;

    /* IEEE allows signed zeros (yuck!) */
    if (x == 0.0) x = 0.0;
    if (!R_FINITE(x)) {
	if(ISNA(x)) snprintf(buff, NB, "%*s", min(w, (NB-1)), CHAR(R_print.na_string));
	else if(ISNAN(x)) snprintf(buff, NB, "%*s", min(w, (NB-1)), "NaN");
	else if(x > 0) snprintf(buff, NB, "%*s", min(w, (NB-1)), "Inf");
	else snprintf(buff, NB, "%*s", min(w, (NB-1)), "-Inf");
    }
    else if (e) {
	if(d) { // '#' flag
	    snprintf(fmt, 20, "%%#%d.%de", min(w, (NB-1)), d);
	}
	else {
	    snprintf(fmt, 20, "%%%d.%de", min(w, (NB-1)), d);
	}
	snprintf(buff, NB, fmt, x);
    }
    else { /* e = 0 */
	snprintf(fmt, 20, "%%%d.%df", min(w, (NB-1)), d);
	snprintf(buff, NB, fmt, x);
    }
    buff[NB-1] = '\0';

    if(strcmp(dec, ".")) { /* replace "." by dec */
	int len = strwidth(dec); /* 3·14 must work */
	if(len != 1) warning(
	    _("the decimal mark is %s than one character wide; this will become an error"),
	    (len > 1) ? "more" : "less");

	char *p, *q;
	for(p = buff, q = buff2; *p; p++) {
	    if(*p == '.') for(const char *r = dec; *r; r++) *q++ = *r;
	    else *q++ = *p;
	}
	*q = '\0';
	out = buff2;
    }

    return out;
}

#define NB3 NB+3
const char
*EncodeComplex(Rcomplex x, int wr, int dr, int er, int wi, int di, int ei,
	       const char *dec)
{
    static char buff[NB3];

    /* IEEE allows signed zeros; strip these here */
    if (x.r == 0.0) x.r = 0.0;
    if (x.i == 0.0) x.i = 0.0;

    if (ISNA(x.r) || ISNA(x.i)) {
	snprintf(buff, NB,
		 "%*s", /* was "%*s%*s", R_print.gap, "", */
		 min(wr+wi+2, (NB-1)), CHAR(R_print.na_string));
    } else {
	char Re[NB];
	const char *Im, *tmp;
	int flagNegIm = 0;
	/* EncodeReal has static buffer, so copy */
	tmp = EncodeReal0(x.r, wr, dr, er, dec);
	strcpy(Re, tmp);
	/* If x.i is very slightly negative, we printed -Oi, and that
	   will often be platform dependent */
	if ( (flagNegIm = (x.i < 0)) ) x.i = -x.i;
	Im = EncodeReal0(x.i, wi, di, ei, dec);
	if (streql(Im, "0")) flagNegIm = FALSE;
	snprintf(buff, NB3, "%s%s%si", Re, flagNegIm ? "-" : "+", Im);
    }
    buff[NB3-1] = '\0';
    return buff;
}

/* robigo: R_print's objects are GC roots (memory.c marks them through
   the global environment and the CHARSXP cache) */
int robigo_print_roots(SEXP *out)
{
    int n = 0;
    if (R_print.na_string) out[n++] = R_print.na_string;
    if (R_print.na_string_noquote) out[n++] = R_print.na_string_noquote;
    if (R_print.env) out[n++] = R_print.env;
    if (R_print.callArgs) out[n++] = R_print.callArgs;
    return n;
}
