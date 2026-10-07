// 64x64 integer matrix multiply, textbook i-j-k order (column walks of B stress the D-cache).
#include "bench.h"
#define N 64
static int A[N][N], B[N][N], C[N][N], D[N][N];
int main(void) {
  for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) { A[i][j] = i + j; B[i][j] = i - j; }
  for (int rep = 0; rep < 2; rep++)
    for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) {
      int s = 0;
      for (int k = 0; k < N; k++) s += A[i][k] * B[k][j];
      C[i][j] = s;
    }
  for (int j = 0; j < N; j++) for (int k = 0; k < N; k++) for (int i = 0; i < N; i++) D[i][j] += A[i][k] * B[k][j];  // j-k-i order as a cross-check
  u64 sum = 0; int bad = 0;
  for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) { sum += (u64)(long)C[i][j]; if (C[i][j] != D[i][j]) bad++; }
  puts_("matmul checksum="); print_dec(sum); puts_(bad ? " MISMATCH\n" : " ok\n");
  return bad != 0;
}
