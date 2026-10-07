// Round-robin vs MLFQ on the same workload: 3 long CPU jobs and 2 interactive jobs (sleep 1 tick,
// run a tiny burst, repeat) start first; 10 ticks later 2 short CPU jobs arrive. Children report
// to the parent through a pipe. MLFQ should let the late short jobs and the interactive jobs
// jump ahead of the (already demoted) long jobs; round-robin treats everybody the same.
#include "user.h"

struct rec { int kind, a, b; };   // kind 0 short, 1 long, 2 interactive (a = total latency, b = max latency)

static void burn(long n) { volatile long x = 0; for (long i = 0; i < n; i++) x += i; }

static void run(int mlfq) {
  vmctl(VM_SETSCHED, mlfq);
  int p[2];
  pipe(p);
  int start = uptime(), njobs = 0;
  for (int i = 0; i < 3; i++, njobs++)           // long jobs
    if (fork() == 0) { burn(1500000); struct rec r = {1, uptime() - start, 0}; write(p[1], &r, sizeof r); exit(0); }
  for (int i = 0; i < 2; i++, njobs++)           // interactive jobs
    if (fork() == 0) {
      int tot = 0, mx = 0;
      for (int k = 0; k < 15; k++) {
        int t0 = uptime();
        sleep(1);
        int lat = uptime() - t0 - 1;             // ticks spent waiting for the CPU after the timer woke us
        if (lat < 0) lat = 0;
        tot += lat; if (lat > mx) mx = lat;
        burn(3000);
      }
      struct rec r = {2, tot, mx};
      write(p[1], &r, sizeof r);
      exit(0);
    }
  sleep(10);                                     // the long jobs get demoted while we wait
  int arrive = uptime();
  for (int i = 0; i < 2; i++, njobs++)           // short jobs arrive late
    if (fork() == 0) { burn(150000); struct rec r = {0, uptime() - arrive, 0}; write(p[1], &r, sizeof r); exit(0); }
  close(p[1]);
  int st;
  for (int i = 0; i < njobs; i++) wait(&st);
  int ns = 0, nl = 0, ni = 0, ts = 0, tl = 0, ilat = 0, imax = 0;
  struct rec r;
  while (read(p[0], &r, sizeof r) == sizeof r) {
    if (r.kind == 0) { ns++; ts += r.a; }
    else if (r.kind == 1) { nl++; tl += r.a; }
    else { ni++; ilat += r.a; if (r.b > imax) imax = r.b; }
  }
  close(p[0]);
  int total = uptime() - start;
  printf("RESULT sched policy=%s short_turnaround_x10=%d long_turnaround_x10=%d interactive_wait_x100=%d interactive_max_wait=%d total_ticks=%d\n",
         mlfq ? "MLFQ" : "RR", ns ? ts * 10 / ns : 0, nl ? tl * 10 / nl : 0, ni ? ilat * 100 / (ni * 15) : 0, imax, total);
}

int main(void) {
  run(0);
  run(1);
  return 0;
}
