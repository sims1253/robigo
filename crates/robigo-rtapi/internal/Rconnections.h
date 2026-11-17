/* robigo's Rconnections.h for GNU R's base-package natives (the A5 note's
 * decisions C and H): GNU R 4.6.2's src/include/Rconnections.h is the spec.
 * A connection is GNU R's public struct Rconn (R_ext/Connections.h, version
 * 1), which robigo-rt's connection holds as its own state: its fields are
 * the connection's, and its methods call the runtime. */
#ifndef R_CONNECTIONS_H_
#define R_CONNECTIONS_H_

#include <stddef.h>
#include <Rinternals.h>
#include <R_ext/Connections.h>
#include <R_ext/Print.h>

/* GNU R's layout (x86-64, LP64), which robigo-rt's RconnC asserts too */
_Static_assert(R_CONNECTIONS_VERSION == 1, "R_CONNECTIONS_VERSION");
_Static_assert(sizeof(struct Rconn) == 488, "sizeof(struct Rconn)");
_Static_assert(offsetof(struct Rconn, class) == 0, "Rconn.class");
_Static_assert(offsetof(struct Rconn, description) == 8, "Rconn.description");
_Static_assert(offsetof(struct Rconn, enc) == 16, "Rconn.enc");
_Static_assert(offsetof(struct Rconn, mode) == 20, "Rconn.mode");
_Static_assert(offsetof(struct Rconn, text) == 28, "Rconn.text");
_Static_assert(offsetof(struct Rconn, isopen) == 32, "Rconn.isopen");
_Static_assert(offsetof(struct Rconn, incomplete) == 36, "Rconn.incomplete");
_Static_assert(offsetof(struct Rconn, canread) == 40, "Rconn.canread");
_Static_assert(offsetof(struct Rconn, canwrite) == 44, "Rconn.canwrite");
_Static_assert(offsetof(struct Rconn, canseek) == 48, "Rconn.canseek");
_Static_assert(offsetof(struct Rconn, blocking) == 52, "Rconn.blocking");
_Static_assert(offsetof(struct Rconn, isGzcon) == 56, "Rconn.isGzcon");
_Static_assert(offsetof(struct Rconn, open) == 64, "Rconn.open");
_Static_assert(offsetof(struct Rconn, close) == 72, "Rconn.close");
_Static_assert(offsetof(struct Rconn, destroy) == 80, "Rconn.destroy");
_Static_assert(offsetof(struct Rconn, vfprintf) == 88, "Rconn.vfprintf");
_Static_assert(offsetof(struct Rconn, fgetc) == 96, "Rconn.fgetc");
_Static_assert(offsetof(struct Rconn, fgetc_internal) == 104, "Rconn.fgetc_internal");
_Static_assert(offsetof(struct Rconn, seek) == 112, "Rconn.seek");
_Static_assert(offsetof(struct Rconn, truncate) == 120, "Rconn.truncate");
_Static_assert(offsetof(struct Rconn, fflush) == 128, "Rconn.fflush");
_Static_assert(offsetof(struct Rconn, read) == 136, "Rconn.read");
_Static_assert(offsetof(struct Rconn, write) == 144, "Rconn.write");
_Static_assert(offsetof(struct Rconn, nPushBack) == 152, "Rconn.nPushBack");
_Static_assert(offsetof(struct Rconn, posPushBack) == 156, "Rconn.posPushBack");
_Static_assert(offsetof(struct Rconn, PushBack) == 160, "Rconn.PushBack");
_Static_assert(offsetof(struct Rconn, save) == 168, "Rconn.save");
_Static_assert(offsetof(struct Rconn, save2) == 172, "Rconn.save2");
_Static_assert(offsetof(struct Rconn, encname) == 176, "Rconn.encname");
_Static_assert(offsetof(struct Rconn, inconv) == 280, "Rconn.inconv");
_Static_assert(offsetof(struct Rconn, outconv) == 288, "Rconn.outconv");
_Static_assert(offsetof(struct Rconn, iconvbuff) == 296, "Rconn.iconvbuff");
_Static_assert(offsetof(struct Rconn, oconvbuff) == 321, "Rconn.oconvbuff");
_Static_assert(offsetof(struct Rconn, next) == 376, "Rconn.next");
_Static_assert(offsetof(struct Rconn, init_out) == 384, "Rconn.init_out");
_Static_assert(offsetof(struct Rconn, navail) == 410, "Rconn.navail");
_Static_assert(offsetof(struct Rconn, inavail) == 412, "Rconn.inavail");
_Static_assert(offsetof(struct Rconn, EOF_signalled) == 416, "Rconn.EOF_signalled");
_Static_assert(offsetof(struct Rconn, UTF8out) == 420, "Rconn.UTF8out");
_Static_assert(offsetof(struct Rconn, id) == 424, "Rconn.id");
_Static_assert(offsetof(struct Rconn, ex_ptr) == 432, "Rconn.ex_ptr");
_Static_assert(offsetof(struct Rconn, private) == 440, "Rconn.private");
_Static_assert(offsetof(struct Rconn, status) == 448, "Rconn.status");
_Static_assert(offsetof(struct Rconn, buff) == 456, "Rconn.buff");
_Static_assert(offsetof(struct Rconn, buff_len) == 464, "Rconn.buff_len");
_Static_assert(offsetof(struct Rconn, buff_stored_len) == 472, "Rconn.buff_stored_len");
_Static_assert(offsetof(struct Rconn, buff_pos) == 480, "Rconn.buff_pos");

int Rconn_fgetc(Rconnection con);
Rconnection getConnection(int n);

#endif
