// Physical page allocator and Sv39 virtual memory management.
#include "kernel.h"

extern char etext[], end[];

// ---- page allocator: intrusive free list ----------------------------------
struct run { struct run *next; };
static struct run *freelist;
static uint64 nfree;
uint64 cow_faults;
// Reference count per physical page (needed for copy-on-write sharing).
static uchar pgref[(PHYSTOP - KERNBASE) / PGSIZE];
#define PGIDX(pa) (((uint64)(pa) - KERNBASE) / PGSIZE)

void kfree(void *pa) {
  if (((uint64)pa % PGSIZE) != 0 || (char *)pa < end || (uint64)pa >= PHYSTOP) panic("kfree");
  if (pgref[PGIDX(pa)] == 0) panic("kfree: double free");
  if (--pgref[PGIDX(pa)] > 0) return;   // still shared by another address space
  memset(pa, 1, PGSIZE);                // catch use-after-free
  struct run *r = pa;
  r->next = freelist; freelist = r; nfree++;
}

void *kalloc(void) {
  struct run *r = freelist;
  if (r) { freelist = r->next; nfree--; pgref[PGIDX(r)] = 1; }
  return r;
}

uint64 kfreepages(void) { return nfree; }
int kpgref(uint64 pa) { return pgref[PGIDX(pa)]; }

void kinit(void) {
  for (char *p = (char *)PGROUNDUP((uint64)end); p + PGSIZE <= (char *)PHYSTOP; p += PGSIZE) {
    struct run *r = (struct run *)p;   // DRAM starts zeroed, so no junk-fill needed here
    r->next = freelist; freelist = r; nfree++;
  }
}

// ---- page tables -------------------------------------------------------------
static pagetable_t kernel_pagetable;

pte_t *walk(pagetable_t pt, uint64 va, int alloc) {
  for (int level = 2; level > 0; level--) {
    pte_t *pte = &pt[PX(level, va)];
    if (*pte & PTE_V) pt = (pagetable_t)PTE2PA(*pte);
    else {
      if (!alloc || (pt = (pagetable_t)kalloc()) == 0) return 0;
      memset(pt, 0, PGSIZE);
      *pte = PA2PTE(pt) | PTE_V;
    }
  }
  return &pt[PX(0, va)];
}

int mappages(pagetable_t pt, uint64 va, uint64 size, uint64 pa, int perm) {
  uint64 a = PGROUNDDOWN(va), last = PGROUNDDOWN(va + size - 1);
  for (;;) {
    pte_t *pte = walk(pt, a, 1);
    if (!pte) return -1;
    if (*pte & PTE_V) panic("mappages: remap");
    // Kernel mappings get A/D preset; user pages leave them to the (hardware) page walker
    // so the swapper can read the Accessed bit for Clock / LRU.
    *pte = PA2PTE(pa) | perm | PTE_V | ((perm & PTE_U) ? 0 : (PTE_A | PTE_D));
    if (a == last) break;
    a += PGSIZE; pa += PGSIZE;
  }
  return 0;
}

static void kvmmap(uint64 va, uint64 pa, uint64 sz, int perm) {
  if (mappages(kernel_pagetable, va, sz, pa, perm) != 0) panic("kvmmap");
}

// Kernel mappings are identity mappings, supervisor-only. Every process page
// table shares the root entries that cover them (VPN2 = 0 and 2).
// Map [va, va+sz) with 2 MiB leaf PTEs (Sv39 level-1 leaves). The emulator caches these in
// its small superpage TLB, so a handful of entries cover the whole 128 MiB direct map.
static void kvmmap_mega(uint64 va, uint64 pa, uint64 sz, int perm) {
  for (uint64 a = va; a < va + sz; a += (1UL << 21), pa += (1UL << 21)) {
    pte_t *p2 = &kernel_pagetable[PX(2, a)];
    if (!(*p2 & PTE_V)) {
      pagetable_t l1 = (pagetable_t)kalloc();
      if (!l1) panic("kvmmap_mega");
      memset(l1, 0, PGSIZE);
      *p2 = PA2PTE(l1) | PTE_V;
    }
    pte_t *p1 = &((pagetable_t)PTE2PA(*p2))[PX(1, a)];
    if (*p1 & PTE_V) panic("kvmmap_mega: remap");
    *p1 = PA2PTE(pa) | perm | PTE_V | PTE_A | PTE_D;
  }
}

void kvminit(void) {
  kernel_pagetable = (pagetable_t)kalloc();
  memset(kernel_pagetable, 0, PGSIZE);
  kvmmap(UART0, UART0, PGSIZE, PTE_R | PTE_W);
  kvmmap(VIRTIO0, VIRTIO0, PGSIZE, PTE_R | PTE_W);
  kvmmap(FINISHER, FINISHER, PGSIZE, PTE_R | PTE_W);  // power-off register
  kvmmap(PLIC, PLIC, 0x400000, PTE_R | PTE_W);
  kvmmap(KERNBASE, KERNBASE, (uint64)etext - KERNBASE, PTE_R | PTE_X);   // text: read+execute only
#if KERNEL_SUPERPAGES
  if ((uint64)end >= KERNBASE + (2UL << 20)) panic("kernel image larger than 2 MiB");
  // The kernel image lives in the first 2 MiB (4 KiB pages keep text read-only);
  // everything above it -- free memory, user frames, page tables -- uses 2 MiB leaves.
  kvmmap((uint64)etext, (uint64)etext, KERNBASE + (2UL << 20) - (uint64)etext, PTE_R | PTE_W);
  kvmmap_mega(KERNBASE + (2UL << 20), KERNBASE + (2UL << 20), PHYSTOP - KERNBASE - (2UL << 20), PTE_R | PTE_W);
#else
  kvmmap((uint64)etext, (uint64)etext, PHYSTOP - (uint64)etext, PTE_R | PTE_W);
#endif
}

void kvminithart(void) {
  sfence_vma();
  w_satp(MAKE_SATP(kernel_pagetable));
  sfence_vma();
}

pagetable_t uvmcreate(void) {
  pagetable_t pt = (pagetable_t)kalloc();
  if (!pt) return 0;
  memset(pt, 0, PGSIZE);
  pt[0] = kernel_pagetable[0];
  pt[2] = kernel_pagetable[2];
  return pt;
}

static void uvmunmap(pagetable_t pt, uint64 va, uint64 npages, int do_free) {
  for (uint64 a = va; a < va + npages * PGSIZE; a += PGSIZE) {
    pte_t *pte = walk(pt, a, 0);
    if (!pte) continue;
    if (!(*pte & PTE_V)) {
      if (*pte & PTE_SWAP) { swap_free(*pte >> 10); *pte = 0; }   // release the swap slot
      continue;
    }
    if (do_free) { track_remove(pt, a); kfree((void *)PTE2PA(*pte)); }
    *pte = 0;
  }
  sfence_vma();   // drop cached translations: the frames may be reused immediately
}

uint64 uvmalloc(pagetable_t pt, uint64 oldsz, uint64 newsz, int perm) {
  if (newsz < oldsz) return oldsz;
  for (uint64 a = PGROUNDUP(oldsz); a < newsz; a += PGSIZE) {
    char *mem = kalloc();
    if (!mem) { uvmdealloc(pt, a, oldsz); return 0; }
    memset(mem, 0, PGSIZE);
    if (mappages(pt, a, PGSIZE, (uint64)mem, perm | PTE_U) != 0) {
      kfree(mem); uvmdealloc(pt, a, oldsz); return 0;
    }
  }
  return newsz;
}

uint64 uvmdealloc(pagetable_t pt, uint64 oldsz, uint64 newsz) {
  if (newsz >= oldsz) return oldsz;
  if (PGROUNDUP(newsz) < PGROUNDUP(oldsz))
    uvmunmap(pt, PGROUNDUP(newsz), (PGROUNDUP(oldsz) - PGROUNDUP(newsz)) / PGSIZE, 1);
  return newsz;
}

static void freewalk(pagetable_t pt) {
  for (int i = 0; i < 512; i++) {
    pte_t pte = pt[i];
    if ((pte & PTE_V) && !(pte & (PTE_R | PTE_W | PTE_X))) freewalk((pagetable_t)PTE2PA(pte));
    else if (pte & PTE_V) panic("freewalk: leaf");
  }
  kfree(pt);
}

// Free a user address space: image+heap [UBASE, sz), stack, then the tables
// private to this process (never the shared kernel subtrees).
void uvmfree(pagetable_t pt, uint64 sz) {
  track_remove_all(pt);
  if (sz > UBASE) uvmunmap(pt, UBASE, PGROUNDUP(sz - UBASE) / PGSIZE, 1);
  uvmunmap(pt, USTACKBASE, USTACKPAGES, 1);
  if (pt[1] & PTE_V) freewalk((pagetable_t)PTE2PA(pt[1]));
  kfree(pt);
}

static int copyrange(pagetable_t old, pagetable_t new, uint64 start, uint64 end_) {
  for (uint64 a = start; a < end_; a += PGSIZE) {
    pte_t *pte = walk(old, a, 0);
    if (!pte || !(*pte & PTE_V)) continue;
    uint64 pa = PTE2PA(*pte);
    int flags = *pte & 0x3ff;
    char *mem = kalloc();
    if (!mem) return -1;
    memmove(mem, (char *)pa, PGSIZE);
    if (mappages(new, a, PGSIZE, (uint64)mem, flags & ~(PTE_V | PTE_A | PTE_D)) != 0) { kfree(mem); return -1; }
  }
  return 0;
}

// Share every mapped page with the child. Writable pages become read-only +
// PTE_COW in BOTH address spaces; the first write by either side faults and
// cowfault() gives the writer a private copy.
static int cowrange(pagetable_t old, pagetable_t new, uint64 start, uint64 end_) {
  for (uint64 a = start; a < end_; a += PGSIZE) {
    pte_t *pte = walk(old, a, 0);
    if (!pte || !(*pte & PTE_V)) continue;
    uint64 pa = PTE2PA(*pte), flags = *pte & 0x3ff;
    if (flags & PTE_W) {
      flags = (flags & ~PTE_W) | PTE_COW;
      *pte = PA2PTE(pa) | flags;
    }
    if (mappages(new, a, PGSIZE, pa, flags & ~(PTE_V | PTE_A | PTE_D)) != 0) return -1;
    pgref[PGIDX(pa)]++;
  }
  return 0;
}

int uvmcopy(pagetable_t old, pagetable_t new, uint64 sz) {
  int bad;
  no_evict++;                                     // never evict while pages are being shared
  swap_resident_range(old, UBASE, PGROUNDUP(sz));  // swapped-out pages must be brought back first
  if (COW_FORK)
    bad = cowrange(old, new, UBASE, PGROUNDUP(sz)) < 0 || cowrange(old, new, USTACKBASE, USTACKTOP) < 0;
  else
    bad = copyrange(old, new, UBASE, PGROUNDUP(sz)) < 0 || copyrange(old, new, USTACKBASE, USTACKTOP) < 0;
  no_evict--;
  sfence_vma();   // parent's writable TLB entries are now stale
  if (bad) { uvmfree(new, sz); return -1; }
  return 0;
}

// Handle a store to a copy-on-write page. Returns 0 if resolved, -1 if the
// access was a genuine protection violation (or memory ran out).
int cowfault(pagetable_t pt, uint64 va) {
  if (va < UBASE || va >= USTACKTOP) return -1;
  pte_t *pte = walk(pt, PGROUNDDOWN(va), 0);
  if (!pte || !(*pte & PTE_V) || !(*pte & PTE_U) || !(*pte & PTE_COW)) return -1;
  uint64 pa = PTE2PA(*pte);
  cow_faults++;
  if (pgref[PGIDX(pa)] == 1) {                      // last sharer: just take ownership
    *pte = (*pte | PTE_W) & ~PTE_COW;
  } else {
    char *mem = kalloc();
    if (!mem) return -1;
    memmove(mem, (char *)pa, PGSIZE);
    uint64 flags = ((*pte & 0x3ff) | PTE_W) & ~PTE_COW;
    *pte = PA2PTE(mem) | flags;
    kfree((void *)pa);                              // drop our reference to the shared page
  }
  sfence_vma();
  return 0;
}

uint64 walkaddr(pagetable_t pt, uint64 va) {
  if (va >= USTACKTOP || va < UBASE) return 0;
  pte_t *pte = walk(pt, va, 0);
  if (!pte || !(*pte & PTE_V)) return vmfault(pt, va);   // lazy allocation or swap-in
  if (!(*pte & PTE_U)) return 0;
  return PTE2PA(*pte);
}

int copyout(pagetable_t pt, uint64 dst, char *src, uint64 len) {
  while (len > 0) {
    uint64 va0 = PGROUNDDOWN(dst);
    if (va0 < UBASE || va0 >= USTACKTOP) return -1;
    pte_t *pte = walk(pt, va0, 0);
    if (!pte || !(*pte & PTE_V)) {                    // not resident: lazy allocation or swap-in
      if (!vmfault(pt, va0)) return -1;
      pte = walk(pt, va0, 0);
    }
    if (!(*pte & PTE_U)) return -1;
    if (*pte & PTE_COW) {                            // kernel writes must also break sharing
      if (cowfault(pt, va0) < 0) return -1;
      pte = walk(pt, va0, 0);
    }
    uint64 pa0 = PTE2PA(*pte);
    uint64 n = PGSIZE - (dst - va0);
    if (n > len) n = len;
    memmove((void *)(pa0 + (dst - va0)), src, n);
    len -= n; src += n; dst = va0 + PGSIZE;
  }
  return 0;
}

int copyin(pagetable_t pt, char *dst, uint64 src, uint64 len) {
  while (len > 0) {
    uint64 va0 = PGROUNDDOWN(src), pa0 = walkaddr(pt, va0);
    if (!pa0) return -1;
    uint64 n = PGSIZE - (src - va0);
    if (n > len) n = len;
    memmove(dst, (void *)(pa0 + (src - va0)), n);
    len -= n; dst += n; src = va0 + PGSIZE;
  }
  return 0;
}

int copyinstr(pagetable_t pt, char *dst, uint64 src, uint64 max) {
  int got_null = 0;
  while (!got_null && max > 0) {
    uint64 va0 = PGROUNDDOWN(src), pa0 = walkaddr(pt, va0);
    if (!pa0) return -1;
    uint64 n = PGSIZE - (src - va0);
    if (n > max) n = max;
    char *p = (char *)(pa0 + (src - va0));
    while (n > 0) {
      if (*p == 0) { *dst = 0; got_null = 1; break; }
      *dst = *p; n--; max--; p++; dst++;
    }
    src = va0 + PGSIZE;
  }
  return got_null ? 0 : -1;
}
