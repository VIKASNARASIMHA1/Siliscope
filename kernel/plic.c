#include "kernel.h"
void plicinit(void) {
  *(uint32 *)(PLIC + UART0_IRQ * 4) = 1;
  *(uint32 *)(PLIC + VIRTIO0_IRQ * 4) = 1;
}
void plicinithart(void) {
  *(uint32 *)(PLIC + 0x2080) = (1 << UART0_IRQ) | (1 << VIRTIO0_IRQ);  // S-mode enable, hart 0
  *(uint32 *)(PLIC + 0x201000) = 0;                                      // threshold
}
int plic_claim(void) { return *(uint32 *)(PLIC + 0x201004); }
void plic_complete(int irq) { *(uint32 *)(PLIC + 0x201004) = irq; }
