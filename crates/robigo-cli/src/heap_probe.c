// mimalloc v3, stats.c:mi_process_info reads this same commit counter.
// Use its headers so the wrapper follows the linked allocator's layout.
#include "mimalloc/internal.h"

#ifdef __cplusplus
extern "C" {
#endif
size_t robigo_mimalloc_committed(void) {
#ifdef _WIN32
    // Windows reports process commit through its OS API; retain that metric.
    size_t commit = 0;
    mi_process_info(NULL, NULL, NULL, NULL, NULL, &commit, NULL, NULL);
    return commit;
#else
    const mi_subproc_t* subproc = _mi_subproc_main();
    if (subproc == NULL) return 0;
    return (size_t)mi_atomic_loadi64_relaxed((_Atomic(int64_t)*)&subproc->stats.committed.current);
#endif
}
#ifdef __cplusplus
}
#endif
