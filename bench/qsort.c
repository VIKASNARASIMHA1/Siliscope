// Quicksort of 40,000 pseudo-random integers (data-dependent, hard-to-predict branches).
#include "bench.h"
#define N 40000
static int a[N];
static u64 seed = 12345;
static u32 rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return seed >> 33; }
static void qs(int lo, int hi) {
  while (hi - lo > 12) {
    int p = a[(lo + hi) / 2], i = lo, j = hi;
    while (i <= j) {
      while (a[i] < p) i++;
      while (a[j] > p) j--;
      if (i <= j) { int t = a[i]; a[i] = a[j]; a[j] = t; i++; j--; }
    }
    if (j - lo < hi - i) { qs(lo, j); lo = i; } else { qs(i, hi); hi = j; }
  }
  for (int i = lo + 1; i <= hi; i++) { int v = a[i], j = i - 1; while (j >= lo && a[j] > v) { a[j + 1] = a[j]; j--; } a[j + 1] = v; }
}
int main(void) {
  for (int i = 0; i < N; i++) a[i] = rnd() & 0xfffff;
  qs(0, N - 1);
  int bad = 0;
  for (int i = 1; i < N; i++) if (a[i - 1] > a[i]) bad++;
  puts_("qsort sorted "); print_dec(N); puts_(bad ? " FAILED\n" : " ok\n");
  return bad != 0;
}
