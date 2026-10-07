// Buffer cache: LRU list of BSIZE-byte disk blocks. "busy" is a sleep-lock.
#include "kernel.h"

static struct { struct buf buf[NBUF]; struct buf head; } bcache;

void binit(void) {
  bcache.head.prev = bcache.head.next = &bcache.head;
  for (struct buf *b = bcache.buf; b < bcache.buf + NBUF; b++) {
    b->next = bcache.head.next; b->prev = &bcache.head;
    bcache.head.next->prev = b; bcache.head.next = b;
  }
}

static struct buf *bget(uint dev, uint blockno) {
  struct buf *b;
retry:
  for (b = bcache.head.next; b != &bcache.head; b = b->next) {
    if (b->dev == dev && b->blockno == blockno) {
      if (b->busy) { sleep(b); goto retry; }
      b->busy = 1; b->refcnt++;
      return b;
    }
  }
  for (b = bcache.head.prev; b != &bcache.head; b = b->prev) {
    if (b->refcnt == 0 && !b->busy) {
      b->dev = dev; b->blockno = blockno; b->valid = 0; b->refcnt = 1; b->busy = 1;
      return b;
    }
  }
  panic("bget: no buffers");
}

struct buf *bread(uint dev, uint blockno) {
  struct buf *b = bget(dev, blockno);
  if (!b->valid) { virtio_disk_rw(b, 0); b->valid = 1; }
  return b;
}

void bwrite(struct buf *b) {
  if (!b->busy) panic("bwrite");
  virtio_disk_rw(b, 1);
}

void brelse(struct buf *b) {
  if (!b->busy) panic("brelse");
  b->busy = 0;
  b->refcnt--;
  if (b->refcnt == 0) {
    b->next->prev = b->prev; b->prev->next = b->next;
    b->next = bcache.head.next; b->prev = &bcache.head;
    bcache.head.next->prev = b; bcache.head.next = b;
  }
  wakeup(b);
}

// Overwrite a whole block without reading it first (used for swap-out).
void bwrite_new(uint dev, uint blockno, const void *data) {
  struct buf *b = bget(dev, blockno);
  memmove(b->data, data, BSIZE);
  b->valid = 1;
  bwrite(b);
  brelse(b);
}
