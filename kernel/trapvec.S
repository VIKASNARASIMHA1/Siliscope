# Trap entry/exit, context switch and the machine-mode timer vector.
.section .text

.globl trapvec
.align 4
trapvec:
  csrrw t6, sscratch, t6        # t6 = &trapframe (from user) or 0 (from kernel)
  beqz t6, 1f
  sd x1, 8(t6)
  sd x2, 16(t6)
  sd x3, 24(t6)
  sd x4, 32(t6)
  sd x5, 40(t6)
  sd x6, 48(t6)
  sd x7, 56(t6)
  sd x8, 64(t6)
  sd x9, 72(t6)
  sd x10, 80(t6)
  sd x11, 88(t6)
  sd x12, 96(t6)
  sd x13, 104(t6)
  sd x14, 112(t6)
  sd x15, 120(t6)
  sd x16, 128(t6)
  sd x17, 136(t6)
  sd x18, 144(t6)
  sd x19, 152(t6)
  sd x20, 160(t6)
  sd x21, 168(t6)
  sd x22, 176(t6)
  sd x23, 184(t6)
  sd x24, 192(t6)
  sd x25, 200(t6)
  sd x26, 208(t6)
  sd x27, 216(t6)
  sd x28, 224(t6)
  sd x29, 232(t6)
  sd x30, 240(t6)
  csrr t5, sscratch
  sd t5, 248(t6)                # user's t6
  csrw sscratch, zero
  csrr t5, sepc
  sd t5, 256(t6)
  csrr t5, sstatus
  sd t5, 264(t6)
  ld sp, 272(t6)                # kernel stack
  mv a0, t6
  call usertrap
1:  # trap taken while already in the kernel (idle loop)
  csrrw t6, sscratch, t6        # undo the swap
  addi sp, sp, -256
  sd x1, 0(sp)
  sd x3, 16(sp)
  sd x4, 24(sp)
  sd x5, 32(sp)
  sd x6, 40(sp)
  sd x7, 48(sp)
  sd x8, 56(sp)
  sd x9, 64(sp)
  sd x10, 72(sp)
  sd x11, 80(sp)
  sd x12, 88(sp)
  sd x13, 96(sp)
  sd x14, 104(sp)
  sd x15, 112(sp)
  sd x16, 120(sp)
  sd x17, 128(sp)
  sd x18, 136(sp)
  sd x19, 144(sp)
  sd x20, 152(sp)
  sd x21, 160(sp)
  sd x22, 168(sp)
  sd x23, 176(sp)
  sd x24, 184(sp)
  sd x25, 192(sp)
  sd x26, 200(sp)
  sd x27, 208(sp)
  sd x28, 216(sp)
  sd x29, 224(sp)
  sd x30, 232(sp)
  sd x31, 240(sp)
  call kerneltrap
  ld x1, 0(sp)
  ld x3, 16(sp)
  ld x4, 24(sp)
  ld x5, 32(sp)
  ld x6, 40(sp)
  ld x7, 48(sp)
  ld x8, 56(sp)
  ld x9, 64(sp)
  ld x10, 72(sp)
  ld x11, 80(sp)
  ld x12, 88(sp)
  ld x13, 96(sp)
  ld x14, 104(sp)
  ld x15, 112(sp)
  ld x16, 120(sp)
  ld x17, 128(sp)
  ld x18, 136(sp)
  ld x19, 144(sp)
  ld x20, 152(sp)
  ld x21, 160(sp)
  ld x22, 168(sp)
  ld x23, 176(sp)
  ld x24, 184(sp)
  ld x25, 192(sp)
  ld x26, 200(sp)
  ld x27, 208(sp)
  ld x28, 216(sp)
  ld x29, 224(sp)
  ld x30, 232(sp)
  ld x31, 240(sp)
  addi sp, sp, 256
  sret

.globl userret
userret:                         # a0 = &trapframe
  mv t6, a0
  ld x1, 8(t6)
  ld x2, 16(t6)
  ld x3, 24(t6)
  ld x4, 32(t6)
  ld x5, 40(t6)
  ld x6, 48(t6)
  ld x7, 56(t6)
  ld x8, 64(t6)
  ld x9, 72(t6)
  ld x10, 80(t6)
  ld x11, 88(t6)
  ld x12, 96(t6)
  ld x13, 104(t6)
  ld x14, 112(t6)
  ld x15, 120(t6)
  ld x16, 128(t6)
  ld x17, 136(t6)
  ld x18, 144(t6)
  ld x19, 152(t6)
  ld x20, 160(t6)
  ld x21, 168(t6)
  ld x22, 176(t6)
  ld x23, 184(t6)
  ld x24, 192(t6)
  ld x25, 200(t6)
  ld x26, 208(t6)
  ld x27, 216(t6)
  ld x28, 224(t6)
  ld x29, 232(t6)
  ld x30, 240(t6)
  ld t6, 248(t6)
  sret

.globl swtch
swtch:                           # swtch(old, new)
  sd ra, 0(a0)
  sd sp, 8(a0)
  sd s0, 16(a0)
  sd s1, 24(a0)
  sd s2, 32(a0)
  sd s3, 40(a0)
  sd s4, 48(a0)
  sd s5, 56(a0)
  sd s6, 64(a0)
  sd s7, 72(a0)
  sd s8, 80(a0)
  sd s9, 88(a0)
  sd s10, 96(a0)
  sd s11, 104(a0)
  ld ra, 0(a1)
  ld sp, 8(a1)
  ld s0, 16(a1)
  ld s1, 24(a1)
  ld s2, 32(a1)
  ld s3, 40(a1)
  ld s4, 48(a1)
  ld s5, 56(a1)
  ld s6, 64(a1)
  ld s7, 72(a1)
  ld s8, 80(a1)
  ld s9, 88(a1)
  ld s10, 96(a1)
  ld s11, 104(a1)
  ret

.globl timervec
.align 4
timervec:                       # machine-mode timer interrupt
  csrrw a0, mscratch, a0
  sd a1, 0(a0)
  sd a2, 8(a0)
  sd a3, 16(a0)
  ld a1, 24(a0)                 # &mtimecmp
  ld a2, 32(a0)                 # interval
  ld a3, 0(a1)
  add a3, a3, a2
  sd a3, 0(a1)
  li a1, 2
  csrw sip, a1                  # raise a supervisor software interrupt
  ld a3, 16(a0)
  ld a2, 8(a0)
  ld a1, 0(a0)
  csrrw a0, mscratch, a0
  mret
