// Supervisor-mode entry: bring up every subsystem, then run the scheduler.
#include "kernel.h"

void kmain(void) {
  consoleinit();
  printf("\nrvsim kernel: booting (RV64, Sv39, MLFQ scheduler)\n");
  kinit();
  printf("mem: %lu free pages (%lu KiB)\n", kfreepages(), kfreepages() * 4);
  kvminit();
  kvminithart();
  printf("vm: paging enabled (Sv39)\n");
  procinit();
  trapinit();
  plicinit();
  plicinithart();
  binit();
  virtio_disk_init();
  userinit();
  printf("init: starting scheduler\n");
  scheduler();
}
