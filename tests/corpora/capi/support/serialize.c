// Serialization from C: R_InitOutPStream over a raw connection, with the
// write callbacks R_WriteConnection provides.
// R: main/serialize.c:R_Serialize, connections
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Connections.h>

static void capi_outbytes(R_outpstream_t stream, void *buf, int length) {
    Rconnection con = (Rconnection)stream->data;
    R_WriteConnection(con, buf, (size_t)length);
}

static void capi_outchar(R_outpstream_t stream, int c) {
    char ch = (char)c;
    capi_outbytes(stream, &ch, 1);
}

SEXP capi_serialize(SEXP obj, SEXP conn) {
    struct R_outpstream_st out;
    R_InitOutPStream(&out, (R_pstream_data_t)R_GetConnection(conn),
                     R_pstream_xdr_format, 3, capi_outchar, capi_outbytes,
                     NULL, R_NilValue);
    R_Serialize(obj, &out);
    return R_NilValue;
}
