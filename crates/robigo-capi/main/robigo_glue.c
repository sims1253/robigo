/* robigo: the parts of GNU R 4.5.0's src/main/util.c the vendored graphics
   engine calls (verbatim), the UTF-8-locale form of mbtoucs, and the
   display-list recording of .External.graphics / .Call.graphics
   (dotcode.c do_Externalgr / do_dotcallgr) around robigo's foreign calls. */
#ifdef HAVE_CONFIG_H
# include <config.h>
#endif
#include <Defn.h>
#include <R_ext/GraphicsEngine.h>
#include <rlocale.h>
#include <errno.h>
#include <R_ext/Riconv.h>

/* R: util.c strIsASCII */
Rboolean strIsASCII(const char *str)
{
    const char *p;
    for(p = str; *p; p++)
	if((unsigned int)*p > 0x7F) return FALSE;
    return TRUE;
}

static R_wchar_t
utf16toucs(wchar_t high, wchar_t low)
{
    return 0x10000 + ((int) (high & 0x3FF) << 10 ) + (int) (low & 0x3FF);
}
static wchar_t
utf8toutf16low(const char *s)
{
    return (unsigned int) LOW_SURROGATE_START | ((s[2] & 0x0F) << 6) | (s[3] & 0x3F);
}
R_wchar_t
utf8toucs32(wchar_t high, const char *s)
{
    return utf16toucs(high, utf8toutf16low(s));
}

/* R: util.c utf8toucs */
size_t
utf8toucs(wchar_t *wc, const char *s)
{
    unsigned int byte;
    wchar_t local, *w;
    byte = *((unsigned char *)s);
    w = wc ? wc: &local;

    if (byte == 0) {
	*w = (wchar_t) 0;
	return 0;
    } else if (byte < 0x80) {
	*w = (wchar_t) byte;
	return 1;
    } else if (byte < 0xC0) {
	return (size_t)-1;
    } else if (byte < 0xE0) {
	if(strlen(s) < 2) return (size_t)-2;
	if ((s[1] & 0xC0) == 0x80) {
	    *w = (wchar_t) (((byte & 0x1F) << 6) | (s[1] & 0x3F));
	    return 2;
	} else return (size_t)-1;
    } else if (byte < 0xF0) {
	if(strlen(s) < 3) return (size_t)-2;
	if (((s[1] & 0xC0) == 0x80) && ((s[2] & 0xC0) == 0x80)) {
	    *w = (wchar_t) (((byte & 0x0F) << 12)
			    | (unsigned int) ((s[1] & 0x3F) << 6)
			    | (s[2] & 0x3F));
	    byte = (unsigned int) *w;
	    /* Surrogates range */
	    if(byte >= 0xD800 && byte <= 0xDFFF) return (size_t)-1;
	    if(byte == 0xFFFE || byte == 0xFFFF) return (size_t)-1;
	    return 3;
	} else return (size_t)-1;

    } else if (byte < 0xf8) {
	if(strlen(s) < 4) return (size_t)-2;
	if (((s[1] & 0xC0) == 0x80) && ((s[2] & 0xC0) == 0x80) && ((s[3] & 0xC0) == 0x80)) {
	    unsigned int cvalue = (((byte & 0x0F) << 18)
			| (unsigned int) ((s[1] & 0x3F) << 12)
			| (unsigned int) ((s[2] & 0x3F) << 6)
			| (s[3] & 0x3F));
	    if(sizeof(wchar_t) < 4) /* Assume UTF-16 and return high surrogate.  Users need to call utf8toutf16low next. */
		*w = (wchar_t) ((cvalue - 0x10000) >> 10) | 0xD800;
	    else
		*w = (wchar_t) cvalue;
	    return 4;
	} else return (size_t)-1;
    }
    if(sizeof(wchar_t) < 4) return (size_t)-2;
    /* So now handle 5.6 byte sequences with no testing */
    if (byte < 0xFC) {
	if(strlen(s) < 5) return (size_t)-2;
	*w = (wchar_t) (((byte & 0x0F) << 24)
			| (unsigned int) ((s[1] & 0x3F) << 12)
			| (unsigned int) ((s[2] & 0x3F) << 12)
			| (unsigned int) ((s[3] & 0x3F) << 6)
			| (s[4] & 0x3F));
	return 5;
    } else {
	if(strlen(s) < 6) return (size_t)-2;
	*w = (wchar_t) (((byte & 0x0F) << 30)
			| (unsigned int) ((s[1] & 0x3F) << 24)
			| (unsigned int) ((s[2] & 0x3F) << 18)
			| (unsigned int) ((s[3] & 0x3F) << 12)
			| (unsigned int) ((s[4] & 0x3F) << 6)
			| (s[5] & 0x3F));
	return 6;
    }
}

/* R: util.c utf8clen: number of additional bytes */
static const unsigned char utf8_table4[] = {
  1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
  1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
  2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
  3,3,3,3,3,3,3,3,4,4,4,4,5,5,5,5 };

int utf8clen(char c)
{
    /* This allows through 8-bit chars 10xxxxxx, which are invalid */
    if ((c & 0xc0) != 0xc0) return 1;
    return 1 + utf8_table4[c & 0x3f];
}

/* R: util.c utf8towcs */
/* despite its name this translates to UTF-16 if there are (invalid)
 * UTF-8 codings for surrogates in the input */
size_t
utf8towcs(wchar_t *wc, const char *s, size_t n)
{
    ssize_t m, res = 0;
    const char *t;
    wchar_t *p;
    wchar_t local;

    if(wc)
	for(p = wc, t = s; ; p++, t += m) {
	    m  = (ssize_t) utf8toucs(p, t);
	    if (m < 0) error(_("invalid input '%s' in 'utf8towcs'"), s);
	    if (m == 0) break;
	    res ++;
	    if (res >= n) break;
	    if (IS_HIGH_SURROGATE(*p)) {
		*(++p) = utf8toutf16low(t);
		res ++;
		if (res >= n) break;
	    }
	}
    else
	for(t = s; ; t += m) {
	    m  = (ssize_t) utf8toucs(&local, t);
	    if (m < 0) error(_("invalid input '%s' in 'utf8towcs'"), s);
	    if (m == 0) break;
	    res ++;
	    if (IS_HIGH_SURROGATE(local))
		res ++;
	}
    return (size_t) res;
}

/* R: util.c mbcsValid */
Rboolean mbcsValid(const char *str)
{
    return  ((int)mbstowcs(NULL, str, 0) >= 0);
}

/* R: util.c mbcsToUcs2 */
# ifdef WORDS_BIGENDIAN
static const char UCS2ENC[] = "UCS-2BE";
# else
static const char UCS2ENC[] = "UCS-2LE";
# endif

/*
 * out=NULL returns the number of the MBCS chars
 */
/* Note: this does not terminate out, as all current uses are to look
 * at 'out' a wchar at a time, and sometimes just one char.
 */
size_t mbcsToUcs2(const char *in, R_ucs2_t *out, int nout, int enc)
{
    void   *cd = NULL ;
    const char *i_buf;
    char *o_buf;
    size_t  i_len, o_len, status, wc_len;
    /* out length */
    wc_len = (enc == CE_UTF8)? utf8towcs(NULL, in, 0) : mbstowcs(NULL, in, 0);
    if (out == NULL || (int)wc_len < 0) return wc_len;

    if ((void*)-1 == (cd = Riconv_open(UCS2ENC, (enc == CE_UTF8) ? "UTF-8": "")))
	return (size_t) -1;

    i_buf = (char *)in;
    i_len = strlen(in); /* not including terminator */
    o_buf = (char *)out;
    o_len = ((size_t) nout) * sizeof(R_ucs2_t);
    status = Riconv(cd, &i_buf, (size_t *)&i_len, &o_buf, (size_t *)&o_len);
    int serrno = errno;
    Riconv_close(cd);
    if (status == (size_t)-1) {
	switch(serrno){
	case EINVAL:
	    return (size_t) -2;
	case EILSEQ:
	    return (size_t) -1;
	case E2BIG:
	    break;
	default:
	    errno = EILSEQ;
	    return (size_t) -1;
	}
    }
    return wc_len; /* status would be better? */
}

/* R: util.c mbtoucs, in robigo's UTF-8 locale (iconv from "" to UCS-4
   decodes one UTF-8 character) */
size_t mbtoucs(unsigned int *wc, const char *s, size_t n)
{
    wchar_t w;
    if(s[0] == 0) {*wc = 0; return 1;}
    size_t used = utf8toucs(&w, s);
    if (used == (size_t) -1 || used == (size_t) -2) return used;
    *wc = (unsigned int) w;
    return 1;
}

/* R: util.c s2u, AdobeSymbol2ucs2 */
static int s2u[224] = {
    0x0020, 0x0021, 0x2200, 0x0023, 0x2203, 0x0025, 0x0026, 0x220D,
    0x0028, 0x0029, 0x2217, 0x002B, 0x002C, 0x2212, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037,
    0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F,
    0x2245, 0x0391, 0x0392, 0x03A7, 0x0394, 0x0395, 0x03A6, 0x0393,
    0x0397, 0x0399, 0x03D1, 0x039A, 0x039B, 0x039C, 0x039D, 0x039F,
    0x03A0, 0x0398, 0x03A1, 0x03A3, 0x03A4, 0x03A5, 0x03C2, 0x03A9,
    0x039E, 0x03A8, 0x0396, 0x005B, 0x2234, 0x005D, 0x22A5, 0x005F,
    0xF8E5, 0x03B1, 0x03B2, 0x03C7, 0x03B4, 0x03B5, 0x03C6, 0x03B3,
    0x03B7, 0x03B9, 0x03D5, 0x03BA, 0x03BB, 0x03BC, 0x03BD, 0x03BF,
    0x03C0, 0x03B8, 0x03C1, 0x03C3, 0x03C4, 0x03C5, 0x03D6, 0x03C9,
    0x03BE, 0x03C8, 0x03B6, 0x007B, 0x007C, 0x007D, 0x223C, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x20AC, 0x03D2, 0x2032, 0x2264, 0x2044, 0x221E, 0x0192, 0x2663,
    0x2666, 0x2665, 0x2660, 0x2194, 0x2190, 0x2191, 0x2192, 0x2193,
    0x00B0, 0x00B1, 0x2033, 0x2265, 0x00D7, 0x221D, 0x2202, 0x2022,
    0x00F7, 0x2260, 0x2261, 0x2248, 0x2026, 0xF8E6, 0xF8E7, 0x21B5,
    0x2135, 0x2111, 0x211C, 0x2118, 0x2297, 0x2295, 0x2205, 0x2229,
    0x222A, 0x2283, 0x2287, 0x2284, 0x2282, 0x2286, 0x2208, 0x2209,
    0x2220, 0x2207, 0xF6DA, 0xF6D9, 0xF6DB, 0x220F, 0x221A, 0x22C5,
    0x00AC, 0x2227, 0x2228, 0x21D4, 0x21D0, 0x21D1, 0x21D2, 0x21D3,
    0x25CA, 0x2329, 0xF8E8, 0xF8E9, 0xF8EA, 0x2211, 0xF8EB, 0xF8EC,
    0xF8ED, 0xF8EE, 0xF8EF, 0xF8F0, 0xF8F1, 0xF8F2, 0xF8F3, 0xF8F4,
    0x0020, 0x232A, 0x222B, 0x2320, 0xF8F5, 0x2321, 0xF8F6, 0xF8F7,
    0xF8F8, 0xF8F9, 0xF8FA, 0xF8FB, 0xF8FC, 0xF8FD, 0xF8FE, 0x0020
};

int Rf_AdobeSymbol2ucs2(int n)
{
    if(n >= 32 && n < 256) return s2u[n-32];
    else return 0;
}

/* R: raw.c mbrtoint */
int mbrtoint(int *w, const char *s)
{
    unsigned int byte;
    byte = *((unsigned char *)s);

    if (byte == 0) {
	*w = 0;
	return 0;
    } else if (byte < 0xC0) {
	*w = (int) byte;
	return 1;
    } else if (byte < 0xE0) {
	if (!s[1]) return -2;
	if ((s[1] & 0xC0) == 0x80) {
	    *w = (int) (((byte & 0x1F) << 6) | (s[1] & 0x3F));
	    return 2;
	} else return -1;
    } else if (byte < 0xF0) {
	if (!s[1] || !s[2]) return -2;
	if (((s[1] & 0xC0) == 0x80) && ((s[2] & 0xC0) == 0x80)) {
	    *w = (int) (((byte & 0x0F) << 12)
			| ((s[1] & 0x3F) << 6) | (s[2] & 0x3F));
	    byte = *w;
	    if (byte >= 0xD800 && byte <= 0xDFFF) return -1; /* surrogate */
	    // Following Corrigendum 9, these are valid in UTF-8
//	    if (byte == 0xFFFE || byte == 0xFFFF) return -1;
	    return 3;
	} else return -1;
    } else if (byte <= 0xF4) { // for RFC3629
	if (!s[1] || !s[2] || !s[3]) return -2;
	if (((s[1] & 0xC0) == 0x80)
	    && ((s[2] & 0xC0) == 0x80)
	    && ((s[3] & 0xC0) == 0x80)) {
	    *w = (int) (((byte & 0x07) << 18)
			| ((s[1] & 0x3F) << 12)
			| ((s[2] & 0x3F) << 6)
			| (s[3] & 0x3F));
	    byte = *w;
	    return (byte <= 0x10FFFF) ? 4 : -1;
	} else return -1;
    } else return -1;
    /* return -2; not reached */
}

/* R: util.c utf8Valid */
#include "valid_utf8.h"
Rboolean utf8Valid(const char *str)
{
    return valid_utf8(str, strlen(str)) == 0;
}

/* R: util.c s2unicode, Rf_AdobeSymbol2utf8, Rf_utf8toAdobeSymbol,
   Rf_utf8Toutf8NoPUA (cairo's symbol-font text) */
static int s2unicode[224] = {
    0x0020, 0x0021, 0x2200, 0x0023, 0x2203, 0x0025, 0x0026, 0x220D,
    0x0028, 0x0029, 0x2217, 0x002B, 0x002C, 0x2212, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037,
    0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F,
    0x2245, 0x0391, 0x0392, 0x03A7, 0x0394, 0x0395, 0x03A6, 0x0393,
    0x0397, 0x0399, 0x03D1, 0x039A, 0x039B, 0x039C, 0x039D, 0x039F,
    0x03A0, 0x0398, 0x03A1, 0x03A3, 0x03A4, 0x03A5, 0x03C2, 0x03A9,
    0x039E, 0x03A8, 0x0396, 0x005B, 0x2234, 0x005D, 0x22A5, 0x005F,
    0x23AF, 0x03B1, 0x03B2, 0x03C7, 0x03B4, 0x03B5, 0x03C6, 0x03B3,
    0x03B7, 0x03B9, 0x03D5, 0x03BA, 0x03BB, 0x03BC, 0x03BD, 0x03BF,
    0x03C0, 0x03B8, 0x03C1, 0x03C3, 0x03C4, 0x03C5, 0x03D6, 0x03C9,
    0x03BE, 0x03C8, 0x03B6, 0x007B, 0x007C, 0x007D, 0x223C, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020, 0x0020,
    0x20AC, 0x03D2, 0x2032, 0x2264, 0x2044, 0x221E, 0x0192, 0x2663,
    0x2666, 0x2665, 0x2660, 0x2194, 0x2190, 0x2191, 0x2192, 0x2193,
    0x00B0, 0x00B1, 0x2033, 0x2265, 0x00D7, 0x221D, 0x2202, 0x2022,
    0x00F7, 0x2260, 0x2261, 0x2248, 0x2026, 0x23D0, 0x23AF, 0x21B5,
    0x2135, 0x2111, 0x211C, 0x2118, 0x2297, 0x2295, 0x2205, 0x2229,
    0x222A, 0x2283, 0x2287, 0x2284, 0x2282, 0x2286, 0x2208, 0x2209,
    0x2220, 0x2207, 0x00AE, 0x00A9, 0x2122, 0x220F, 0x221A, 0x22C5,
    0x00AC, 0x2227, 0x2228, 0x21D4, 0x21D0, 0x21D1, 0x21D2, 0x21D3,
    0x25CA, 0x2329, 0x00AE, 0x00A9, 0x2122, 0x2211, 0x239B, 0x239C,
    0x239D, 0x23A1, 0x23A2, 0x23A3, 0x23A7, 0x23A8, 0x23A9, 0x23AA,
    0x0020, 0x232A, 0x222B, 0x2320, 0x23AE, 0x2321, 0x239E, 0x239F,
    0x23A0, 0x23A4, 0x23A5, 0x23A6, 0x23AB, 0x23AC, 0x23AD, 0x0020
};

void *Rf_AdobeSymbol2utf8(char *work, const char *c0, size_t nwork,
			  Rboolean usePUA)
{
    const unsigned char *c = (unsigned char *) c0;
    unsigned char *t = (unsigned char *) work;
    while (*c) {
	if (*c < 32) *t++ = ' ';
	else {
	    unsigned int u;
	    if (usePUA) {
		u = (unsigned int) s2u[*c - 32];
	    } else {
		u = (unsigned int) s2unicode[*c - 32];
	    }
	    if (u < 128) *t++ = (unsigned char) u;
	    else if (u < 0x800) {
		*t++ = (unsigned char) (0xc0 | (u >> 6));
		*t++ = (unsigned char) (0x80 | (u & 0x3f));
	    } else {
		*t++ = (unsigned char) (0xe0 | (u >> 12));
		*t++ = (unsigned char) (0x80 | ((u >> 6) & 0x3f));
		*t++ = (unsigned char) (0x80 | (u & 0x3f));
	    }
	}
	if (t+6 > (unsigned char *)(work + nwork)) break;
	c++;
    }
    *t = '\0';
    return (char*) work;
}

int Rf_utf8toAdobeSymbol(char *out, const char *in) {
    int i, j, k, used, tmp, nc = 0, found;
    int *symbolint;
    const char *s = in;
    const char *p = in;
    for ( ; *p; p += utf8clen(*p)) nc++;
    symbolint = (int *) R_alloc(nc, sizeof(int));
    for (i = 0, j = 0; i < nc; i++, j++) {
	/* Convert UTF8 to int */
	used = mbrtoint(&tmp, s);
	if (used < 0)
	    error(_("invalid UTF-8 string"));
	symbolint[j] = tmp;
	found = 0;
	/* Convert int to CE_SYMBOL char */
	for (k = 0; k < 224; k++) {
	    if (symbolint[j] == s2u[k]) {
		out[j] = (char)(k + 32);
		found = 1;
	    }
	    if (found) break;
	}
	if (!found)
	    error(_("Conversion failed"));
	s += used;
    }
    out[nc] = '\0';
    return nc;
}

const char* Rf_utf8Toutf8NoPUA(const char *in)
{
    int i, j, used, tmp;
    /* At least enough because assumes each incoming char only one byte */
    int nChar = 3*(int)strlen(in) + 1;
    char *result = R_alloc(nChar, sizeof(char));
    const char *s = in;
    char *p = result;
    for (i = 0; i < nChar; i++) {
	/* Convert UTF8 char to int */
	used = mbrtoint(&tmp, s);
	/* Only re-encode if necessary
	 * This is more efficient AND protects against input that is
	 * NOT from Rf_AdobeSymbol2utf8(), e.g., plotmath on Windows
	 * (which is from reEnc(CE_LATIN1, CE_UTF8))
	 */
	if (tmp > 0xF600) {
	    char inChar[4], symbolChar[2], utf8Char[4];
	    char *q;
	    for (j = 0; j < used; j++) {
		inChar[j] = *s++;
	    }
	    inChar[used] = '\0';
	    Rf_utf8toAdobeSymbol(symbolChar, inChar);
	    Rf_AdobeSymbol2utf8(utf8Char, symbolChar, 4, FALSE);
	    q = utf8Char;
	    while (*q) {
		*p++ = *q++;
	    }
	} else {
	    for (j = 0; j < used; j++) {
		*p++ = *s++;
	    }
	}
    }
    *p = '\0';
    return result;
}

/* R: dotcode.c do_Externalgr / do_dotcallgr, split around the call
   (called as .Call routines, so that errors unwind to the host): before,
   the current device's recordGraphics is saved and cleared ... */
SEXP robigo_gr_begin(void)
{
    pGEDevDesc dd = GEcurrentDevice();
    bool record = dd->recordGraphics;
    dd->recordGraphics = false;
    return ScalarLogical(record);
}

/* ... after, restored, and the operation recorded on the display list */
SEXP robigo_gr_end(SEXP call, SEXP op, SEXP args, SEXP record)
{
    pGEDevDesc dd = GEcurrentDevice();
    dd->recordGraphics = asLogical(record);
    if (GErecording(call, dd)) { // which is record && call != R_NilValue
	if (!GEcheckState(dd))
	    errorcall(call, _("invalid graphics state"));
	GErecordGraphicOperation(op, args, dd);
    }
    return R_NilValue;
}

/* R: memory.c RunGenCollect: the devices' display lists, snapshots and
   event environments are roots */
int robigo_gr_roots(SEXP *out, int max)
{
    int n = 0;
    for (int i = 0; i < R_MaxDevices; i++) {
	pGEDevDesc gdd = GEgetDevice(i);
	if (gdd && n + 3 <= max) {
	    if (gdd->displayList) out[n++] = gdd->displayList;
	    if (gdd->savedSnapshot) out[n++] = gdd->savedSnapshot;
	    if (gdd->dev && gdd->dev->eventEnv) out[n++] = gdd->dev->eventEnv;
	}
    }
    return n;
}
