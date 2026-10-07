// Table-driven CRC-32 over a 256 KiB buffer, cross-checked against a bitwise version.
#include "bench.h"
#define LEN (256 * 1024)
static u8 buf[LEN];
static u32 table[256];
static u32 crc_bitwise(const u8 *p, int n) {
  u32 c = 0xffffffff;
  for (int i = 0; i < n; i++) { c ^= p[i]; for (int k = 0; k < 8; k++) c = (c >> 1) ^ (0xedb88320 & -(c & 1)); }
  return ~c;
}
int main(void) {
  for (u32 i = 0; i < 256; i++) { u32 c = i; for (int k = 0; k < 8; k++) c = (c >> 1) ^ (0xedb88320 & -(c & 1)); table[i] = c; }
  u32 x = 1;
  for (int i = 0; i < LEN; i++) { x = x * 1664525 + 1013904223; buf[i] = x >> 24; }
  u32 c = 0xffffffff;
  for (int pass = 0; pass < 2; pass++) { c = 0xffffffff; for (int i = 0; i < LEN; i++) c = table[(c ^ buf[i]) & 0xff] ^ (c >> 8); }
  c = ~c;
  u32 ref = crc_bitwise(buf, 4096), t = 0xffffffff;
  for (int i = 0; i < 4096; i++) t = table[(t ^ buf[i]) & 0xff] ^ (t >> 8);
  int ok = (~t == ref);
  puts_("crc32="); print_hex(c); puts_(ok ? " ok\n" : " WRONG\n");
  return !ok;
}
