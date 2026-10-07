// exec(): load an ELF64 executable from the file system into a fresh address space.
#include "kernel.h"

#define ELF_MAGIC 0x464C457FU
struct elfhdr {
  uint magic; uchar elf[12]; ushort type, machine; uint version;
  uint64 entry, phoff, shoff; uint flags;
  ushort ehsize, phentsize, phnum, shentsize, shnum, shstrndx;
};
struct proghdr { uint type, flags; uint64 off, vaddr, paddr, filesz, memsz, align; };
#define PT_LOAD 1

// Copy `sz` bytes of the file (from `offset`) to user virtual address `va`;
// segments need not be page aligned (several can share a page).
static int loadseg(pagetable_t pt, uint64 va, struct inode *ip, uint offset, uint sz) {
  for (uint i = 0; i < sz;) {
    uint64 cur = va + i;
    uint64 pa = walkaddr(pt, PGROUNDDOWN(cur));
    if (pa == 0) panic("loadseg: address should exist");
    uint inpage = cur % PGSIZE;
    uint n = PGSIZE - inpage;
    if (n > sz - i) n = sz - i;
    if (readi(ip, 0, pa + inpage, offset + i, n) != (int)n) return -1;
    i += n;
  }
  return 0;
}

int exec(char *path, char **argv) {
  struct proc *p = curproc;
  struct inode *ip;
  struct elfhdr elf;
  struct proghdr ph;
  pagetable_t pt = 0;
  uint64 sz = UBASE;

  if ((ip = namei(path)) == 0) return -1;
  ilock(ip);
  if (readi(ip, 0, (uint64)&elf, 0, sizeof(elf)) != sizeof(elf) || elf.magic != ELF_MAGIC) goto bad;
  if ((pt = uvmcreate()) == 0) goto bad;

  for (int i = 0, off = elf.phoff; i < elf.phnum; i++, off += sizeof(ph)) {
    if (readi(ip, 0, (uint64)&ph, off, sizeof(ph)) != sizeof(ph)) goto bad;
    if (ph.type != PT_LOAD) continue;
    if (ph.memsz < ph.filesz || ph.vaddr + ph.memsz < ph.vaddr) goto bad;
    if (ph.vaddr < UBASE || ph.vaddr + ph.memsz >= USTACKBASE - PGSIZE) goto bad;
    uint64 nsz = uvmalloc(pt, sz, ph.vaddr + ph.memsz, PTE_R | PTE_W | PTE_X);
    if (nsz == 0) goto bad;
    sz = nsz;
    if (loadseg(pt, ph.vaddr, ip, ph.off, ph.filesz) < 0) goto bad;
  }
  iunlockput(ip);
  ip = 0;

  sz = PGROUNDUP(sz);
  if (uvmalloc(pt, USTACKBASE, USTACKTOP, PTE_R | PTE_W) == 0) goto bad;

  // Push argument strings, then the argv[] array (all kernel-side pointers on entry).
  uint64 sp = USTACKTOP, ustack[MAXARG + 1];
  int argc;
  for (argc = 0; argv[argc]; argc++) {
    if (argc >= MAXARG) goto bad;
    sp -= strlen(argv[argc]) + 1;
    sp &= ~15UL;
    if (copyout(pt, sp, argv[argc], strlen(argv[argc]) + 1) < 0) goto bad;
    ustack[argc] = sp;
  }
  ustack[argc] = 0;
  sp -= (argc + 1) * sizeof(uint64);
  sp &= ~15UL;
  if (copyout(pt, sp, (char *)ustack, (argc + 1) * sizeof(uint64)) < 0) goto bad;

  // Commit.
  char *s = path, *last = path;
  for (; *s; s++) if (*s == '/') last = s + 1;
  safestrcpy(p->name, last, sizeof(p->name));
  pagetable_t oldpt = p->pagetable;
  uint64 oldsz = p->sz;
  p->pagetable = pt;
  p->sz = sz;
  p->tf.sepc = elf.entry;
  p->tf.x[2] = sp;
  p->tf.x[10] = argc;
  p->tf.x[11] = sp;
  w_satp(MAKE_SATP(pt));
  sfence_vma();
  uvmfree(oldpt, oldsz);
  return argc;

bad:
  if (pt) uvmfree(pt, sz);
  if (ip) iunlockput(ip);
  return -1;
}
