// Naive recursive Fibonacci: call/return heavy, stack traffic, tiny working set.
#include "bench.h"
static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main(void) {
  int r = fib(27);
  puts_("fib(27)="); print_dec(r); puts_(r == 196418 ? " ok\n" : " WRONG\n");
  return r != 196418;
}
