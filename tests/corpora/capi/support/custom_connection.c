// A connection C code makes: R_new_custom_connection, over a string in
// memory. Its methods log their calls, so the order of open, close and
// destroy shows. R: main/connections.c:R_new_custom_connection.
#define R_NO_REMAP
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Connections.h>
#include <string.h>
#include <stdlib.h>

#if !defined(R_CONNECTIONS_VERSION) || R_CONNECTIONS_VERSION != 1
#error "R_ext/Connections.h version 1"
#endif

typedef struct {
    char *data;
    size_t len, pos;
    size_t chunk;   // bytes per read when not blocking (0: all)
    int bad;        // reads fail: a negative count
    Rconnection self;
    char name[16];
} mem;

static Rboolean mem_open(Rconnection con) {
    mem *m = con->private;
    Rprintf("[%s open \"%s\"]\n", m->name, con->mode);
    if (strchr(con->mode, 'w'))
        Rf_error("memconn: can only read");
    m->pos = 0;
    con->text = strchr(con->mode, 'b') ? FALSE : TRUE;
    con->isopen = TRUE;
    con->incomplete = TRUE;
    return TRUE;
}

static void mem_close(Rconnection con) {
    mem *m = con->private;
    Rprintf("[%s close]\n", m->name);
    con->isopen = FALSE;
    con->incomplete = FALSE;
}

static void mem_destroy(Rconnection con) {
    mem *m = con->private;
    Rprintf("[%s destroy]\n", m->name);
    free(m->data);
    free(m);
}

static size_t mem_read(void *target, size_t sz, size_t ni, Rconnection con) {
    mem *m = con->private;
    if (m->bad)
        return (size_t) -1;
    size_t want = sz * ni, left = m->len - m->pos;
    if (!con->blocking && m->chunk && want > m->chunk)
        want = m->chunk;
    if (want > left)
        want = left;
    memcpy(target, m->data + m->pos, want);
    m->pos += want;
    con->incomplete = m->pos < m->len;
    return want / sz;
}

// fgetc_internal only: R's dummy_fgetc (the default fgetc) calls it
static int mem_fgetc_internal(Rconnection con) {
    unsigned char c;
    return mem_read(&c, 1, 1, con) ? c : -1; /* R_EOF */
}

SEXP cc_new(SEXP name, SEXP text, SEXP chunk) {
    Rconnection con;
    const char *s = CHAR(STRING_ELT(text, 0));
    SEXP rc = PROTECT(R_new_custom_connection(CHAR(STRING_ELT(name, 0)), "r", "memconn", &con));
    mem *m = malloc(sizeof(mem));
    m->len = strlen(s);
    m->data = malloc(m->len + 1);
    memcpy(m->data, s, m->len + 1);
    m->pos = 0;
    int ch = Rf_asInteger(chunk);
    m->bad = ch < 0;
    m->chunk = ch < 0 ? 0 : (size_t) ch;
    m->self = con;
    strncpy(m->name, CHAR(STRING_ELT(name, 0)), sizeof(m->name) - 1);
    m->name[sizeof(m->name) - 1] = '\0';
    con->private = m;
    con->canseek = FALSE;
    con->canwrite = FALSE;
    con->blocking = TRUE;
    con->open = mem_open;
    con->close = mem_close;
    con->destroy = mem_destroy;
    con->read = mem_read;
    con->fgetc_internal = mem_fgetc_internal;
    UNPROTECT(1);
    return rc;
}

// R_GetConnection gives the struct the package filled in, and conn_id's
// address is its id
SEXP cc_check(SEXP sc) {
    Rconnection con = R_GetConnection(sc);
    mem *m = con->private;
    SEXP id = Rf_getAttrib(sc, Rf_install("conn_id"));
    SEXP ans = PROTECT(Rf_allocVector(LGLSXP, 3));
    LOGICAL(ans)[0] = con == m->self;
    LOGICAL(ans)[1] = TYPEOF(id) == EXTPTRSXP && R_ExternalPtrAddr(id) == con->id;
    LOGICAL(ans)[2] = strcmp(con->class, "memconn") == 0 && strcmp(con->description, m->name) == 0;
    UNPROTECT(1);
    return ans;
}

// curl stores its own pointer in conn_id: then the connection is never
// closed when its R object goes
SEXP cc_replace_addr(SEXP sc) {
    static int other;
    R_SetExternalPtrAddr(Rf_getAttrib(sc, Rf_install("conn_id")), &other);
    return R_NilValue;
}

// R_ReadConnection on the package's struct calls its read method
SEXP cc_read(SEXP sc, SEXP n) {
    Rconnection con = R_GetConnection(sc);
    size_t len = (size_t) Rf_asInteger(n);
    SEXP ans = PROTECT(Rf_allocVector(RAWSXP, len));
    size_t got = R_ReadConnection(con, RAW(ans), len);
    SEXP out = Rf_lengthgets(ans, (R_xlen_t) got);
    UNPROTECT(1);
    return out;
}

// C writes a field, then R code reads it in the same .Call: it sees the
// struct (GNU R has only the struct)
SEXP cc_flip_then_eval(SEXP sc, SEXP fun, SEXP rho) {
    Rconnection con = R_GetConnection(sc);
    con->incomplete = !con->incomplete;
    SEXP call = PROTECT(Rf_lang1(fun));
    SEXP ans = Rf_eval(call, rho);
    UNPROTECT(1);
    return ans;
}
