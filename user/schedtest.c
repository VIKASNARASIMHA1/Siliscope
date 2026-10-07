// Scheduler demo: CPU-bound jobs of different lengths plus an interactive job.
// Under MLFQ the short job and the interactive job finish early; under plain
// round-robin everything shares the CPU equally.
#include "user.h"

static void burn(long n) { volatile long x = 0; for (long i = 0; i < n; i++) x += i; }

int main(void) {
  long work[4] = {500000, 1000000, 2000000, 4000000};
  int start = uptime();
  for (int i = 0; i < 4; i++) {
    if (fork() == 0) {
      burn(work[i]);
      printf("  cpu job %d (%lu iterations) finished at tick +%d\n", i, work[i], uptime() - start);
      exit(0);
    }
  }
  if (fork() == 0) {
    for (int k = 0; k < 5; k++) { burn(20000); sleep(1); }
    printf("  interactive job (5 short bursts) finished at tick +%d\n", uptime() - start);
    exit(0);
  }
  int st;
  for (int i = 0; i < 5; i++) wait(&st);
  printf("schedtest: done in %d ticks\n", uptime() - start);
  ps();
  return 0;
}
