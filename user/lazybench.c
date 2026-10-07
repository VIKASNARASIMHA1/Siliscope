// Lazy (demand-paged) sbrk vs eager sbrk: reserve 32 MiB but touch only 1/16 of the pages.
#include "user.h"

static void run(int lazy, int report) {
  vmctl(VM_SETLAZY, lazy);
  long f0 = freepages();
  int pages = 8192;                       // 32 MiB
  char *p = sbrk(pages * 4096);
  for (int i = 0; i < pages; i += 16) p[i * 4096] = 1;   // touch 512 pages
  long used = f0 - freepages();
  int ok = 1;
  for (int i = 0; i < pages; i += 16) if (p[i * 4096] != 1) ok = 0;
  if (lazy && p[4096] != 0) ok = 0;       // an untouched page must read as zero
  sbrk(-(pages * 4096));
  long leak = f0 - freepages();           // page-table pages stay allocated; data frames must all come back
  if (report)
    printf("RESULT lazy mode=%s reserved_pages=%d touched_pages=%d frames_used=%d frames_left_after_free=%d ok=%d\n",
           lazy ? "lazy" : "eager", pages, pages / 16, (int)used, (int)leak, ok);
}

int main(void) {
  run(1, 0);      // warm-up: creates the page-table pages so both measured runs start from the same state
  run(0, 1);
  run(1, 1);
  vmctl(VM_SETLAZY, 1);
  return 0;
}
