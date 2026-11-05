// .Fortran's character arguments: one string, in a buffer of at least 256
// bytes, and one string back. R: main/dotcode.c:do_dotCode (STRSXP, Fort).
#include <string.h>

// a Fortran-callable routine (trailing underscore): the length it got,
// then its first character replaced
void cfort_(char *s, int *n) {
    *n = (int) strlen(s);
    if (*n > 0)
        s[0] = 'X';
}

// overwrites the string it got (the buffer holds it): only its first
// 255 characters come back
void cfortlong_(char *s) {
    memset(s, 'y', strlen(s));
}
