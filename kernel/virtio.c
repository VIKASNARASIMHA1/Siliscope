// virtio-blk driver (legacy MMIO interface), interrupt-driven.
#include "kernel.h"

#define R(r) ((volatile uint32 *)(VIRTIO0 + (r)))
#define VIRTIO_MMIO_MAGIC_VALUE 0x000
#define VIRTIO_MMIO_VERSION 0x004
#define VIRTIO_MMIO_DEVICE_ID 0x008
#define VIRTIO_MMIO_DRIVER_FEATURES 0x020
#define VIRTIO_MMIO_GUEST_PAGE_SIZE 0x028
#define VIRTIO_MMIO_QUEUE_SEL 0x030
#define VIRTIO_MMIO_QUEUE_NUM_MAX 0x034
#define VIRTIO_MMIO_QUEUE_NUM 0x038
#define VIRTIO_MMIO_QUEUE_ALIGN 0x03c
#define VIRTIO_MMIO_QUEUE_PFN 0x040
#define VIRTIO_MMIO_QUEUE_NOTIFY 0x050
#define VIRTIO_MMIO_INTERRUPT_STATUS 0x060
#define VIRTIO_MMIO_INTERRUPT_ACK 0x064
#define VIRTIO_MMIO_STATUS 0x070
#define STATUS_ACK 1
#define STATUS_DRIVER 2
#define STATUS_DRIVER_OK 4
#define STATUS_FEATURES_OK 8
#define NUM 8
#define VRING_DESC_F_NEXT 1
#define VRING_DESC_F_WRITE 2
#define VIRTIO_BLK_T_IN 0
#define VIRTIO_BLK_T_OUT 1

struct virtq_desc { uint64 addr; uint32 len; uint16 flags; uint16 next; };
struct virtq_avail { uint16 flags; uint16 idx; uint16 ring[NUM]; };
struct virtq_used_elem { uint32 id; uint32 len; };
struct virtq_used { uint16 flags; uint16 idx; struct virtq_used_elem ring[NUM]; };
struct virtio_blk_req { uint32 type; uint32 reserved; uint64 sector; };

static struct disk {
  char pages[2 * PGSIZE] __attribute__((aligned(PGSIZE)));
  struct virtq_desc *desc;
  struct virtq_avail *avail;
  struct virtq_used *used;
  char free[NUM];
  uint16 used_idx;
  struct { struct buf *b; char status; } info[NUM];
  struct virtio_blk_req ops[NUM];
} disk;

void virtio_disk_init(void) {
  if (*R(VIRTIO_MMIO_MAGIC_VALUE) != 0x74726976 || *R(VIRTIO_MMIO_VERSION) != 1 || *R(VIRTIO_MMIO_DEVICE_ID) != 2)
    panic("virtio: no disk found (run with --disk build/fs.img)");
  uint32 status = STATUS_ACK;
  *R(VIRTIO_MMIO_STATUS) = status;
  status |= STATUS_DRIVER;
  *R(VIRTIO_MMIO_STATUS) = status;
  *R(VIRTIO_MMIO_DRIVER_FEATURES) = 0;
  status |= STATUS_FEATURES_OK;
  *R(VIRTIO_MMIO_STATUS) = status;
  *R(VIRTIO_MMIO_GUEST_PAGE_SIZE) = PGSIZE;
  *R(VIRTIO_MMIO_QUEUE_SEL) = 0;
  if (*R(VIRTIO_MMIO_QUEUE_PFN) != 0) panic("virtio: queue in use");
  if (*R(VIRTIO_MMIO_QUEUE_NUM_MAX) < NUM) panic("virtio: queue too short");
  *R(VIRTIO_MMIO_QUEUE_NUM) = NUM;
  *R(VIRTIO_MMIO_QUEUE_ALIGN) = PGSIZE;
  memset(disk.pages, 0, sizeof(disk.pages));
  *R(VIRTIO_MMIO_QUEUE_PFN) = ((uint64)disk.pages) >> PGSHIFT;
  disk.desc = (struct virtq_desc *)disk.pages;
  disk.avail = (struct virtq_avail *)(disk.pages + NUM * sizeof(struct virtq_desc));
  disk.used = (struct virtq_used *)(disk.pages + PGSIZE);
  for (int i = 0; i < NUM; i++) disk.free[i] = 1;
  status |= STATUS_DRIVER_OK;
  *R(VIRTIO_MMIO_STATUS) = status;
}

uint64 virtio_disk_sectors(void) { return *R(0x100); }   // capacity (low 32 bits), in 512-byte sectors

static int alloc_desc(void) {
  for (int i = 0; i < NUM; i++) if (disk.free[i]) { disk.free[i] = 0; return i; }
  return -1;
}
static void free_desc(int i) { disk.free[i] = 1; wakeup(&disk.free[0]); }

void virtio_disk_rw(struct buf *b, int write) {
  uint64 sector = b->blockno * (BSIZE / 512);
  int idx[3];
  for (;;) {                       // wait until 3 descriptors are free
    int got = 0;
    for (; got < 3; got++) { idx[got] = alloc_desc(); if (idx[got] < 0) break; }
    if (got == 3) break;
    for (int j = 0; j < got; j++) free_desc(idx[j]);
    sleep(&disk.free[0]);
  }
  struct virtio_blk_req *req = &disk.ops[idx[0]];
  req->type = write ? VIRTIO_BLK_T_OUT : VIRTIO_BLK_T_IN;
  req->reserved = 0;
  req->sector = sector;
  disk.desc[idx[0]] = (struct virtq_desc){(uint64)req, sizeof(*req), VRING_DESC_F_NEXT, idx[1]};
  disk.desc[idx[1]] = (struct virtq_desc){(uint64)b->data, BSIZE, (write ? 0 : VRING_DESC_F_WRITE) | VRING_DESC_F_NEXT, idx[2]};
  disk.info[idx[0]].status = 0xff;
  disk.desc[idx[2]] = (struct virtq_desc){(uint64)&disk.info[idx[0]].status, 1, VRING_DESC_F_WRITE, 0};
  b->disk = 1;
  disk.info[idx[0]].b = b;
  disk.avail->ring[disk.avail->idx % NUM] = idx[0];
  asm volatile("fence rw, rw" ::: "memory");
  disk.avail->idx += 1;
  asm volatile("fence rw, rw" ::: "memory");
  *R(VIRTIO_MMIO_QUEUE_NOTIFY) = 0;
  while (b->disk == 1) sleep(b);
  if (disk.info[idx[0]].status != 0) panic("virtio: disk error");
  disk.info[idx[0]].b = 0;
  for (int i = 0; i < 3; i++) free_desc(idx[i]);
}

void virtio_disk_intr(void) {
  *R(VIRTIO_MMIO_INTERRUPT_ACK) = *R(VIRTIO_MMIO_INTERRUPT_STATUS) & 0x3;
  asm volatile("fence rw, rw" ::: "memory");
  while (disk.used_idx != disk.used->idx) {
    int id = disk.used->ring[disk.used_idx % NUM].id;
    if (disk.info[id].status != 0) panic("virtio_disk_intr status");
    struct buf *b = disk.info[id].b;
    b->disk = 0;
    wakeup(b);
    disk.used_idx++;
  }
}
