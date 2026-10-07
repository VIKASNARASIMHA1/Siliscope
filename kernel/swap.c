// Demand paging and swapping.
//
//  * Lazy sbrk: growing the heap only raises p->sz; a page is allocated (zero-filled)
//    on its first touch, via the page-fault handler.
//  * Swapping: an optional resident-set limit (vmctl(VM_SETLIMIT, n)) caps how many
//    demand-allocated heap pages may be in RAM. When the cap is reached, a victim is
//    chosen by the selected policy (FIFO / Clock / LRU-by-aging / Random), written to a
//    swap slot on the virtio disk, and its PTE becomes an invalid "swapped" entry
//    (V=0, PTE_SWAP=1, slot number in the PPN field). A later access page-faults and
//    the page is read back in.
//  * Only unshared (refcount 1) anonymous pages are ever evicted.
#include "kernel.h"

int memlimit;                 // 0 = swapping disabled
int policy = POL_CLOCK;
int lazy_sbrk = 1;
int no_evict;                 // >0 while fork is sharing pages: never evict
uint64 lazy_faults, swap_ins, swap_outs;
static int swap_used;
static uchar swapmap[SWAPSLOTS], swap_busy[SWAPSLOTS];

struct rpage { pagetable_t pt; uint64 va; uint64 seq; uchar age; };
static struct rpage res[MAXRES];
static int nres, hand;
static uint64 seqno;
static uint rng_state = 2463534242u;

static uint xorshift(void) {
  rng_state ^= rng_state << 13; rng_state ^= rng_state >> 17; rng_state ^= rng_state << 5;
  return rng_state;
}

// ---- resident list ---------------------------------------------------------------
static int track_find(pagetable_t pt, uint64 va) {
  for (int i = 0; i < nres; i++) if (res[i].pt == pt && res[i].va == va) return i;
  return -1;
}

static void track_del(int i) {
  for (int k = i; k < nres - 1; k++) res[k] = res[k + 1];   // keep arrival order (Clock hand)
  nres--;
  if (hand > i) hand--;
  if (hand >= nres) hand = 0;
}

static void track_add(pagetable_t pt, uint64 va) {
  if (nres >= MAXRES || track_find(pt, va) >= 0) return;
  res[nres].pt = pt; res[nres].va = va; res[nres].seq = ++seqno; res[nres].age = 0;
  nres++;
}

void track_remove(pagetable_t pt, uint64 va) {
  if (nres == 0) return;
  int i = track_find(pt, va);
  if (i >= 0) track_del(i);
}

void track_remove_all(pagetable_t pt) {
  for (int i = 0; i < nres;) { if (res[i].pt == pt) track_del(i); else i++; }
}

// ---- swap space ------------------------------------------------------------------
static int swap_alloc(void) {
  for (int i = 0; i < SWAPSLOTS; i++) if (!swapmap[i]) { swapmap[i] = 1; swap_used++; return i; }
  return -1;
}

void swap_free(int slot) {
  while (swap_busy[slot]) sleep(&swap_busy[slot]);   // a write to this slot may still be in flight
  swapmap[slot] = 0;
  swap_used--;
}

static void swap_write(int slot, char *pa) {
  for (int i = 0; i < PGSIZE / BSIZE; i++) bwrite_new(1, SWAPSTART + slot * (PGSIZE / BSIZE) + i, pa + i * BSIZE);
}

static void swap_read(int slot, char *pa) {
  for (int i = 0; i < PGSIZE / BSIZE; i++) {
    struct buf *b = bread(1, SWAPSTART + slot * (PGSIZE / BSIZE) + i);
    memmove(pa + i * BSIZE, b->data, BSIZE);
    brelse(b);
  }
}

// ---- victim selection --------------------------------------------------------------
static int evictable(int i) {
  pte_t *pte = walk(res[i].pt, res[i].va, 0);
  return pte && (*pte & PTE_V) && (*pte & PTE_U) && kpgref(PTE2PA(*pte)) == 1;
}

static int pick_victim(void) {
  int best = -1;
  switch (policy) {
  case POL_FIFO:
    for (int i = 0; i < nres; i++)
      if (evictable(i) && (best < 0 || res[i].seq < res[best].seq)) best = i;
    return best;
  case POL_RANDOM: {
    int start = xorshift() % nres;
    for (int k = 0; k < nres; k++) { int i = (start + k) % nres; if (evictable(i)) return i; }
    return -1;
  }
  case POL_CLOCK: {                                  // second chance using the hardware Accessed bit
    int cleared = 0;
    for (int steps = 0; steps < 2 * nres + 1; steps++) {
      if (hand >= nres) hand = 0;
      int i = hand;
      if (!evictable(i)) { hand = (hand + 1) % nres; continue; }
      pte_t *pte = walk(res[i].pt, res[i].va, 0);
      if (*pte & PTE_A) { *pte &= ~PTE_A; cleared = 1; hand = (hand + 1) % nres; continue; }
      hand = (hand + 1) % nres;
      if (cleared) sfence_vma();
      return i;
    }
    if (cleared) sfence_vma();
    return -1;
  }
  case POL_LRU:                                      // aging approximation of LRU
  default:
    for (int i = 0; i < nres; i++) {
      pte_t *pte = walk(res[i].pt, res[i].va, 0);
      if (!pte || !(*pte & PTE_V)) continue;
      res[i].age = (res[i].age >> 1) | ((*pte & PTE_A) ? 0x80 : 0);
      *pte &= ~PTE_A;
    }
    sfence_vma();                                    // so the next access sets A again
    for (int i = 0; i < nres; i++)
      if (evictable(i) && (best < 0 || res[i].age < res[best].age || (res[i].age == res[best].age && res[i].seq < res[best].seq))) best = i;
    return best;
  }
}

// Evict one page to swap. Returns 0 on success, -1 if nothing could be evicted.
static int evict_one(void) {
  if (no_evict || nres == 0) return -1;
  int v = pick_victim();
  if (v < 0) return -1;
  struct rpage r = res[v];
  int slot = swap_alloc();
  if (slot < 0) return -1;                           // swap full
  track_del(v);
  pte_t *pte = walk(r.pt, r.va, 0);
  uint64 pa = PTE2PA(*pte);
  uint64 flags = *pte & 0x1fe;                       // keep R W X U and the COW bit
  *pte = ((uint64)slot << 10) | PTE_SWAP | flags;    // valid bit cleared: the page is gone
  sfence_vma();
  swap_busy[slot] = 1;                               // frame content is frozen; write it out
  swap_write(slot, (char *)pa);
  swap_busy[slot] = 0;
  wakeup(&swap_busy[slot]);
  kfree((void *)pa);
  swap_outs++;
  return 0;
}

static void make_room(void) {
  if (!memlimit) return;
  while (nres >= memlimit) if (evict_one() < 0) break;
}

// ---- fault handling ------------------------------------------------------------------
static uint64 swap_in(pagetable_t pt, uint64 va, pte_t *pte) {
  int slot = *pte >> 10;
  uint64 flags = *pte & 0x1fe;
  while (swap_busy[slot]) sleep(&swap_busy[slot]);
  make_room();
  char *mem = kalloc();
  if (!mem) { if (evict_one() < 0 || !(mem = kalloc())) return 0; }
  swap_read(slot, mem);
  *pte = PA2PTE(mem) | (flags & ~(PTE_A | PTE_D)) | PTE_V;
  swap_free(slot);
  swap_ins++;
  if (memlimit) track_add(pt, va);
  sfence_vma();
  return (uint64)mem;
}

static uint64 lazy_alloc(pagetable_t pt, uint64 va) {
  make_room();
  char *mem = kalloc();
  if (!mem) { if (evict_one() < 0 || !(mem = kalloc())) return 0; }
  memset(mem, 0, PGSIZE);
  if (mappages(pt, va, PGSIZE, (uint64)mem, PTE_R | PTE_W | PTE_X | PTE_U) != 0) { kfree(mem); return 0; }
  lazy_faults++;
  if (memlimit) track_add(pt, va);
  return (uint64)mem;
}

// Called when va has no valid mapping. Returns the physical address of the page
// once it is resident, or 0 if va is not valid memory for the current process.
uint64 vmfault(pagetable_t pt, uint64 va) {
  struct proc *p = curproc;
  if (!p || p->pagetable != pt) return 0;
  va = PGROUNDDOWN(va);
  if (va < UBASE || va >= USTACKTOP) return 0;
  pte_t *pte = walk(pt, va, 0);
  if (pte && (*pte & PTE_V)) return 0;               // mapped: this is a real protection fault
  if (pte && (*pte & PTE_SWAP)) return swap_in(pt, va, pte);
  if (va >= p->sz) return 0;                         // outside the heap/image
  return lazy_alloc(pt, va);
}

// fork needs every page of the parent to exist: bring swapped pages back (without evicting).
void swap_resident_range(pagetable_t pt, uint64 start, uint64 end) {
  for (uint64 a = start; a < end; a += PGSIZE) {
    pte_t *pte = walk(pt, a, 0);
    if (pte && !(*pte & PTE_V) && (*pte & PTE_SWAP)) swap_in(pt, a, pte);
  }
}

// ---- control / statistics --------------------------------------------------------------
uint64 vmctl(int op, int arg) {
  switch (op) {
  case VM_SETLIMIT:
    if (arg > 0 && virtio_disk_sectors() < (uint64)(SWAPSTART + SWAPSLOTS * (PGSIZE / BSIZE)) * (BSIZE / 512)) {
      printf("swap: this disk image has no swap area (it was built by an older xtask). Delete the target/ and build/ folders and rebuild.\n");
      return -1;
    }
    memlimit = arg < 0 ? 0 : (arg > MAXRES ? MAXRES : arg);
    return memlimit;
  case VM_SETPOLICY: if (arg < 0 || arg > POL_RANDOM) return -1; policy = arg; return policy;
  case VM_GETSTAT:
    switch (arg) {
    case ST_LAZY_FAULTS: return lazy_faults;
    case ST_SWAP_INS: return swap_ins;
    case ST_SWAP_OUTS: return swap_outs;
    case ST_COW_FAULTS: return cow_faults;
    case ST_RESIDENT: return nres;
    case ST_SWAP_USED: return swap_used;
    case ST_LIMIT: return memlimit;
    }
    return -1;
  case VM_RESETSTATS: lazy_faults = swap_ins = swap_outs = 0; cow_faults = 0; return 0;
  case VM_SETSCHED: {
    extern struct proc proc[];
    sched_mlfq = arg != 0;
    for (struct proc *p = proc; p < &proc[NPROC]; p++) { p->prio = 0; p->used = 0; }
    return sched_mlfq;
  }
  case VM_SETLAZY: lazy_sbrk = arg != 0; return lazy_sbrk;
  }
  return -1;
}
