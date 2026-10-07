// Page-replacement policy comparison inside the real kernel.
// A 64-page heap is accessed through a 24-frame resident-set limit; pages that do not fit are
// swapped to the virtio disk. Every access verifies the page contents, so a wrong swap-in is caught.
#include "user.h"

#define WS 64
#define LIMIT 24
#define REFS 2400
#define HOT 12
#define LOOP 30

static uint shadow[WS];
static unsigned short refstr[REFS];
static char linebuf[REFS * 4 + 96];
static uint rng = 12345;
static uint rnd(void) { rng = rng * 1103515245 + 12345; return (rng >> 16) & 0x7fff; }

static char *pol_name[] = {"FIFO", "CLOCK", "LRU-aging", "RANDOM"};
static char *pat_name[] = {"hot-set", "loop"};

static void run(int policy, int pattern) {
  rng = 12345;
  char *base = sbrk(WS * 4096);
  for (int i = 0; i < WS; i++) shadow[i] = 0;
  vmctl(VM_RESETSTATS, 0);
  vmctl(VM_SETPOLICY, policy);
  if (vmctl(VM_SETLIMIT, LIMIT) != LIMIT) { printf("swapbench: no swap area on this disk image\n"); exit(1); }
  int bad = 0;
  for (int i = 0; i < REFS; i++) {
    int pg;
    if (pattern == 0) pg = (rnd() % 100 < 80) ? (int)(rnd() % HOT) : (int)(rnd() % WS);   // 80% of refs hit a small hot set
    else pg = i % LOOP;                                                                    // cyclic scan slightly larger than memory
    refstr[i] = (unsigned short)pg;
    volatile uint *w = (volatile uint *)(base + pg * 4096);
    uint v = *w;
    if (v != shadow[pg]) bad++;
    *w = v + 1;
    shadow[pg] = v + 1;
  }
  int minor = (int)vmctl(VM_GETSTAT, ST_LAZY_FAULTS);
  int major = (int)vmctl(VM_GETSTAT, ST_SWAP_INS);
  int outs = (int)vmctl(VM_GETSTAT, ST_SWAP_OUTS);
  printf("RESULT swap policy=%s pattern=%s refs=%d minor_faults=%d major_faults=%d swap_outs=%d corrupt=%d\n",
         pol_name[policy], pat_name[pattern], REFS, minor, major, outs, bad);
  if (policy == 0) {      // print the exact reference string once, for the offline OPT comparison
    int n = 0;
    char *hdr = "REFSTRING pattern=";
    for (char *c = hdr; *c; c++) linebuf[n++] = *c;
    for (char *c = pat_name[pattern]; *c; c++) linebuf[n++] = *c;
    char *mid = " : ";
    for (char *c = mid; *c; c++) linebuf[n++] = *c;
    for (int i = 0; i < REFS; i++) {
      int v = refstr[i];
      if (v >= 10) linebuf[n++] = '0' + v / 10;
      linebuf[n++] = '0' + v % 10;
      linebuf[n++] = ' ';
    }
    linebuf[n++] = '\n';
    write(1, linebuf, n);
  }
  sbrk(-(WS * 4096));
  vmctl(VM_SETLIMIT, 0);
  if (vmctl(VM_GETSTAT, ST_SWAP_USED) != 0 || vmctl(VM_GETSTAT, ST_RESIDENT) != 0)
    printf("RESULT swap-leak policy=%s slots_in_use=%d tracked=%d\n", pol_name[policy], (int)vmctl(VM_GETSTAT, ST_SWAP_USED), (int)vmctl(VM_GETSTAT, ST_RESIDENT));
}

int main(void) {
  for (int pat = 0; pat < 2; pat++)
    for (int pol = 0; pol < 4; pol++) run(pol, pat);
  return 0;
}
