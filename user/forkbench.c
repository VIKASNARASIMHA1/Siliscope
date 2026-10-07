// Fork cost benchmark: a process with ~2 MiB of touched heap forks 20 children,
// each of which writes a single page and exits. With copy-on-write only that
// one page is copied per fork; with eager copying all ~520 pages are.
#include "user.h"

int main(void) {
  int pages = 512;
  char *p = sbrk(pages * 4096);
  for (int i = 0; i < pages; i++) p[i * 4096] = (char)i;     // make every page resident
  int free0 = freepages();
  int t0 = uptime();
  for (int k = 0; k < 20; k++) {
    int pid = fork();
    if (pid == 0) { p[0] = 2; exit(0); }
    if (k == 0) printf("forkbench: pages consumed by one fork of a 2 MiB process (while child alive): %d\n", free0 - freepages());
    wait(0);
  }
  printf("forkbench: 20 forks done in %d ticks\n", uptime() - t0);
  return 0;
}
