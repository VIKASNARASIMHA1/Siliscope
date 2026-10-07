// Sieve of Eratosthenes up to 1,000,000 (streaming byte accesses, regular branches).
#include "bench.h"
#define N 1000000
static u8 comp[N + 1];
int main(void) {
  for (u64 i = 2; i * i <= N; i++) if (!comp[i]) for (u64 j = i * i; j <= N; j += i) comp[j] = 1;
  u64 count = 0;
  for (int i = 2; i <= N; i++) count += !comp[i];
  puts_("sieve primes<=1e6: "); print_dec(count); puts_(count == 78498 ? " ok\n" : " WRONG\n");
  return count != 78498;
}
