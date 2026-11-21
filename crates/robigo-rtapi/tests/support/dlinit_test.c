/* A shared object whose constructor and destructor stop inside `dlopen`
 * and `dlclose` until robigo-rtapi's dynload test lets them go on
 * (tests/dynload.rs, a_librarys_initializers_and_finalizers_hold_env).
 * Only when the file `<object>.armed` exists: each creates
 * `<object>.ready`, waits for `<object>.go` (60 s at most, so that a failed
 * test never hangs), then removes both. The test checks the environment's
 * lock in between. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

static void handshake(void)
{
    Dl_info i;
    char armed[4096], ready[4096], go[4096];
    struct timespec ms = {0, 1000000};
    int fd;
    if (!dladdr((void *)handshake, &i) || !i.dli_fname)
        return;
    snprintf(armed, sizeof armed, "%s.armed", i.dli_fname);
    snprintf(ready, sizeof ready, "%s.ready", i.dli_fname);
    snprintf(go, sizeof go, "%s.go", i.dli_fname);
    if (access(armed, F_OK) != 0)
        return;
    fd = open(ready, O_WRONLY | O_CREAT, 0600);
    if (fd < 0)
        return;
    close(fd);
    for (int k = 0; k < 60000 && access(go, F_OK) != 0; k++)
        nanosleep(&ms, NULL);
    unlink(ready);
    unlink(go);
}

__attribute__((constructor)) static void dlinit_ctor(void) { handshake(); }
__attribute__((destructor)) static void dlinit_dtor(void) { handshake(); }
