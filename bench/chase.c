// Pointer chasing through a 512 KiB random cycle: every access depends on the last,
// so the result is dominated by cache capacity (L1 miss, L2 behaviour, memory).
#include "bench.h"
#define N 65536
static u64 next[N];
static u64 seed = 99;
static u32 rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return seed >> 33; }
int main(void) {
  for (u64 i = 0; i < N; i++) next[i] = i;
  for (u64 i = N - 1; i > 0; i--) { u64 j = rnd() % i; u64 t = next[i]; next[i] = next[j]; next[j] = t; }  // Sattolo: one single cycle
  u64 p = 0;
  for (int step = 0; step < 4 * N; step++) p = next[p];
  puts_("chase end="); print_dec(p); puts_(p == 0 ? " ok (full cycle)\n" : " WRONG\n");
  return p != 0;
}
