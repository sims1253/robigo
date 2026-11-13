/* The C side of robigo-rtcore's contexts: the layout of RCNTXT as C sees it
 * (include/Defn.h), for the runtime to check its #[repr(C)] declaration
 * against (context.rs, `check_layout`: once per isolate, and in the
 * crate's tests). */
#include <stdalign.h>
#include <Defn.h>

#define F(f) offsetof(RCNTXT, f)

/* sizeof, alignof, then the offset of every field in declaration order;
 * context.rs lists the same fields in the same order. */
const size_t robigo_rcntxt_layout[] = {
    sizeof(RCNTXT),
    alignof(RCNTXT),
    F(nextcontext),
    F(callflag),
    F(cjmpbuf),
    F(cstacktop),
    F(evaldepth),
    F(promargs),
    F(callfun),
    F(sysparent),
    F(call),
    F(cloenv),
    F(conexit),
    F(cend),
    F(cenddata),
    F(vmax),
    F(intsusp),
    F(gcenabled),
    F(bcintactive),
    F(bcbody),
    F(bcpc),
    F(relpc),
    F(handlerstack),
    F(restartstack),
    F(prstack),
    F(nodestack),
    F(bcprottop),
    F(bcframe),
    F(srcref),
    F(browserfinish),
    F(returnValue),
    F(jumptarget),
    F(jumpmask),
    F(robigo),
    F(robigo.flags),
    F(robigo.nargs),
    F(robigo.fillargs),
    F(robigo.filldata),
    F(robigo.parked),
    F(robigo.parkedkind),
    F(robigo.parkedmask),
    F(robigo.parkedtarget),
    F(robigo.parkedvalue),
    F(robigo.parkedcondition),
    F(robigo.parkedcall),
    sizeof(R_bcstack_t),
    offsetof(R_bcstack_t, u),
};

const size_t robigo_rcntxt_layout_len = sizeof robigo_rcntxt_layout / sizeof robigo_rcntxt_layout[0];

_Static_assert(sizeof(((RCNTXT *) 0)->cjmpbuf) == sizeof(sigjmp_buf), "cjmpbuf is a sigjmp_buf");
_Static_assert(sizeof(R_bcstack_t) == 16, "R_bcstack_t is GNU R's 16 bytes on LP64");
