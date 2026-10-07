// On-disk file system: superblock, inodes (12 direct + 1 indirect), bitmap,
// directories. Layout matches tools written in xtask/src/mkfs.rs.
#include "kernel.h"

static struct superblock sb;
static struct { struct inode inode[NINODE]; } icache;

void fsinit(int dev) {
  struct buf *bp = bread(dev, 1);
  memmove(&sb, bp->data, sizeof(sb));
  brelse(bp);
  if (sb.magic != FSMAGIC) panic("fsinit: bad file system magic");
  printf("fs: %u blocks, %u inodes\n", sb.size, sb.ninodes);
}

// ---- blocks ------------------------------------------------------------------
static void bzero(int dev, int bno) {
  struct buf *bp = bread(dev, bno);
  memset(bp->data, 0, BSIZE);
  bwrite(bp);
  brelse(bp);
}

static uint balloc(uint dev) {
  for (uint b = 0; b < sb.size; b += BPB) {
    struct buf *bp = bread(dev, BBLOCK(b, sb));
    for (uint bi = 0; bi < BPB && b + bi < sb.size; bi++) {
      int m = 1 << (bi % 8);
      if ((bp->data[bi / 8] & m) == 0) {
        bp->data[bi / 8] |= m;
        bwrite(bp);
        brelse(bp);
        bzero(dev, b + bi);
        return b + bi;
      }
    }
    brelse(bp);
  }
  printf("balloc: out of blocks\n");
  return 0;
}

static void bfree(int dev, uint b) {
  struct buf *bp = bread(dev, BBLOCK(b, sb));
  int bi = b % BPB, m = 1 << (bi % 8);
  if ((bp->data[bi / 8] & m) == 0) panic("bfree: block already free");
  bp->data[bi / 8] &= ~m;
  bwrite(bp);
  brelse(bp);
}

// ---- inodes ------------------------------------------------------------------
static struct inode *iget(uint dev, uint inum);
struct inode *ialloc(uint dev, short type) {
  for (int inum = 1; inum < (int)sb.ninodes; inum++) {
    struct buf *bp = bread(dev, IBLOCK(inum, sb));
    struct dinode *dip = (struct dinode *)bp->data + inum % IPB;
    if (dip->type == 0) {
      memset(dip, 0, sizeof(*dip));
      dip->type = type;
      bwrite(bp);
      brelse(bp);
      return iget(dev, inum);
    }
    brelse(bp);
  }
  printf("ialloc: no inodes on disk\n");
  return 0;
}

void iupdate(struct inode *ip) {
  struct buf *bp = bread(ip->dev, IBLOCK(ip->inum, sb));
  struct dinode *dip = (struct dinode *)bp->data + ip->inum % IPB;
  dip->type = ip->type; dip->major = ip->major; dip->minor = ip->minor;
  dip->nlink = ip->nlink; dip->size = ip->size;
  memmove(dip->addrs, ip->addrs, sizeof(ip->addrs));
  bwrite(bp);
  brelse(bp);
}

static struct inode *iget(uint dev, uint inum) {
  struct inode *empty = 0;
  for (struct inode *ip = icache.inode; ip < icache.inode + NINODE; ip++) {
    if (ip->ref > 0 && ip->dev == dev && ip->inum == inum) { ip->ref++; return ip; }
    if (!empty && ip->ref == 0) empty = ip;
  }
  if (!empty) panic("iget: no inodes");
  empty->dev = dev; empty->inum = inum; empty->ref = 1; empty->valid = 0; empty->busy = 0;
  return empty;
}

struct inode *idup(struct inode *ip) { ip->ref++; return ip; }

void ilock(struct inode *ip) {
  if (!ip || ip->ref < 1) panic("ilock");
  while (ip->busy) sleep(ip);
  ip->busy = 1;
  if (!ip->valid) {
    struct buf *bp = bread(ip->dev, IBLOCK(ip->inum, sb));
    struct dinode *dip = (struct dinode *)bp->data + ip->inum % IPB;
    ip->type = dip->type; ip->major = dip->major; ip->minor = dip->minor;
    ip->nlink = dip->nlink; ip->size = dip->size;
    memmove(ip->addrs, dip->addrs, sizeof(ip->addrs));
    brelse(bp);
    ip->valid = 1;
    if (ip->type == 0) panic("ilock: no type");
  }
}

void iunlock(struct inode *ip) {
  if (!ip || !ip->busy || ip->ref < 1) panic("iunlock");
  ip->busy = 0;
  wakeup(ip);
}

static void itrunc(struct inode *ip) {
  for (int i = 0; i < NDIRECT; i++) if (ip->addrs[i]) { bfree(ip->dev, ip->addrs[i]); ip->addrs[i] = 0; }
  if (ip->addrs[NDIRECT]) {
    struct buf *bp = bread(ip->dev, ip->addrs[NDIRECT]);
    uint *a = (uint *)bp->data;
    for (uint j = 0; j < NINDIRECT; j++) if (a[j]) bfree(ip->dev, a[j]);
    brelse(bp);
    bfree(ip->dev, ip->addrs[NDIRECT]);
    ip->addrs[NDIRECT] = 0;
  }
  ip->size = 0;
  iupdate(ip);
}

void iput(struct inode *ip) {
  if (ip->ref == 1 && ip->valid && ip->nlink == 0) {
    while (ip->busy) sleep(ip);
    ip->busy = 1;
    itrunc(ip);
    ip->type = 0;
    iupdate(ip);
    ip->valid = 0;
    ip->busy = 0;
    wakeup(ip);
  }
  ip->ref--;
}

void iunlockput(struct inode *ip) { iunlock(ip); iput(ip); }

static uint bmap(struct inode *ip, uint bn) {
  uint addr;
  if (bn < NDIRECT) {
    if ((addr = ip->addrs[bn]) == 0) { addr = balloc(ip->dev); if (!addr) return 0; ip->addrs[bn] = addr; }
    return addr;
  }
  bn -= NDIRECT;
  if (bn < NINDIRECT) {
    if ((addr = ip->addrs[NDIRECT]) == 0) { addr = balloc(ip->dev); if (!addr) return 0; ip->addrs[NDIRECT] = addr; }
    struct buf *bp = bread(ip->dev, addr);
    uint *a = (uint *)bp->data;
    if ((addr = a[bn]) == 0) {
      addr = balloc(ip->dev);
      if (addr) { a[bn] = addr; bwrite(bp); }
    }
    brelse(bp);
    return addr;
  }
  panic("bmap: out of range");
}

void stati(struct inode *ip, struct stat *st) {
  st->dev = ip->dev; st->ino = ip->inum; st->type = ip->type; st->nlink = ip->nlink; st->size = ip->size;
}

static int dst_copy(int user, uint64 dst, void *src, uint n) {
  if (user) return copyout(curproc->pagetable, dst, src, n);
  memmove((void *)dst, src, n);
  return 0;
}
static int src_copy(int user, void *dst, uint64 src, uint n) {
  if (user) return copyin(curproc->pagetable, dst, src, n);
  memmove(dst, (void *)src, n);
  return 0;
}

int readi(struct inode *ip, int user_dst, uint64 dst, uint off, uint n) {
  if (off > ip->size || off + n < off) return 0;
  if (off + n > ip->size) n = ip->size - off;
  uint tot = 0, m;
  for (; tot < n; tot += m, off += m, dst += m) {
    uint addr = bmap(ip, off / BSIZE);
    if (addr == 0) break;
    struct buf *bp = bread(ip->dev, addr);
    m = BSIZE - off % BSIZE;
    if (m > n - tot) m = n - tot;
    int bad = dst_copy(user_dst, dst, bp->data + off % BSIZE, m);
    brelse(bp);
    if (bad < 0) return -1;
  }
  return tot;
}

int writei(struct inode *ip, int user_src, uint64 src, uint off, uint n) {
  if (off > ip->size || off + n < off) return -1;
  if (off + n > MAXFILE * BSIZE) return -1;
  uint tot = 0, m;
  for (; tot < n; tot += m, off += m, src += m) {
    uint addr = bmap(ip, off / BSIZE);
    if (addr == 0) break;
    struct buf *bp = bread(ip->dev, addr);
    m = BSIZE - off % BSIZE;
    if (m > n - tot) m = n - tot;
    if (src_copy(user_src, bp->data + off % BSIZE, src, m) < 0) { brelse(bp); break; }
    bwrite(bp);
    brelse(bp);
  }
  if (off > ip->size) ip->size = off;
  iupdate(ip);
  return tot;
}

// ---- directories ---------------------------------------------------------------
static int namecmp(const char *s, const char *t) { return strncmp(s, t, DIRSIZ); }

struct inode *dirlookup(struct inode *dp, char *name, uint *poff) {
  if (dp->type != T_DIR) panic("dirlookup: not a dir");
  struct dirent de;
  for (uint off = 0; off < dp->size; off += sizeof(de)) {
    if (readi(dp, 0, (uint64)&de, off, sizeof(de)) != sizeof(de)) panic("dirlookup read");
    if (de.inum == 0) continue;
    if (namecmp(name, de.name) == 0) {
      if (poff) *poff = off;
      return iget(dp->dev, de.inum);
    }
  }
  return 0;
}

int dirlink(struct inode *dp, char *name, uint inum) {
  struct inode *ip;
  if ((ip = dirlookup(dp, name, 0)) != 0) { iput(ip); return -1; }
  struct dirent de;
  uint off;
  for (off = 0; off < dp->size; off += sizeof(de)) {
    if (readi(dp, 0, (uint64)&de, off, sizeof(de)) != sizeof(de)) panic("dirlink read");
    if (de.inum == 0) break;
  }
  strncpy(de.name, name, DIRSIZ);
  de.inum = inum;
  if (writei(dp, 0, (uint64)&de, off, sizeof(de)) != sizeof(de)) return -1;
  return 0;
}

static char *skipelem(char *path, char *name) {
  while (*path == '/') path++;
  if (*path == 0) return 0;
  char *s = path;
  while (*path != '/' && *path != 0) path++;
  int len = path - s;
  if (len >= DIRSIZ) memmove(name, s, DIRSIZ);
  else { memmove(name, s, len); name[len] = 0; }
  while (*path == '/') path++;
  return path;
}

static struct inode *namex(char *path, int parent, char *name) {
  struct inode *ip, *next;
  if (*path == '/') ip = iget(1, ROOTINO);
  else ip = idup(curproc->cwd);
  while ((path = skipelem(path, name)) != 0) {
    ilock(ip);
    if (ip->type != T_DIR) { iunlockput(ip); return 0; }
    if (parent && *path == '\0') { iunlock(ip); return ip; }
    if ((next = dirlookup(ip, name, 0)) == 0) { iunlockput(ip); return 0; }
    iunlockput(ip);
    ip = next;
  }
  if (parent) { iput(ip); return 0; }
  return ip;
}

struct inode *namei(char *path) { char name[DIRSIZ]; return namex(path, 0, name); }
struct inode *nameiparent(char *path, char *name) { return namex(path, 1, name); }
