# Architecture notes

## Boot flow
`rvsim` loads the ELF into DRAM at 0x8000_0000 and starts the hart in M-mode at the entry point.
`kernel/boot.S` sets a stack and calls `start()` (M-mode): delegates all traps to S-mode, installs `timervec`
(the CLINT timer handler that converts `mtimecmp` interrupts into supervisor software interrupts), `mret`s into `kmain()` in S-mode.
`kmain` initialises console, page allocator, kernel page table (Sv39), process table, trap vector, PLIC, buffer cache and the virtio disk,
creates process 1 and enters the scheduler. Process 1 runs `forkret`, which mounts the file system and `exec`s `/init`;
`init` opens the console as fds 0/1/2 and runs `sh`.

## Address spaces
```
0x0000_0000 .. 0x3fff_ffff   VPN2=0  kernel MMIO window (UART, virtio, PLIC, finisher)  S-only
0x4000_0000 .. 0x7fff_ffff   VPN2=1  user: image+heap from 0x4000_0000 up, stack (8 pages) at the top
0x8000_0000 .. 0x87ff_ffff   VPN2=2  kernel text/data + free memory, identity mapped        S-only
```
Root-table entries 0 and 2 are shared by every process, so the kernel is mapped in every address space and no trampoline page is needed:
a trap simply keeps running on the same `satp`. User code cannot touch kernel pages (U=0), and the kernel reaches user memory through
software page-table walks (`copyin/copyout`).

## Trap path (`kernel/trapvec.S`)
`sscratch` holds the process trapframe address while in user mode and 0 while in the kernel. `trapvec` swaps `t6` with `sscratch`:
non-zero means "from user" (save all registers to the trapframe, switch to the per-process kernel stack, call `usertrap`);
zero means "from kernel" (only possible in the scheduler's idle loop; save on the current stack, call `kerneltrap`).
`usertrapret` re-arms `sscratch`, sets `SPP=0`/`SPIE=1`, and `userret` restores registers and `sret`s.

## Scheduler
Single run queue scan, 3 priority levels with slices of 1/2/4 ticks. A process that uses its whole slice is demoted; one that sleeps keeps its level;
every 50 ticks all processes are boosted to level 0. The idle loop enables interrupts and executes `wfi`; the emulator fast-forwards virtual time to the next timer event.

## Copy-on-write fork (`kernel/vm.c`)
`uvmcopy` walks the parent's user pages; for each writable page it clears `PTE_W`, sets the software bit `PTE_COW` (RSW bit 8) in the parent's PTE, maps the same physical page into the child with the same flags and increments `pgref[page]`
(then `sfence.vma`, because the parent's cached writable translations are stale). A store to such a page raises scause 15; `usertrap` calls `cowfault()`:
if `pgref == 1` the page is simply made writable again, otherwise a new page is allocated, the contents copied, the PTE re-pointed at the copy and the old page's refcount dropped via `kfree`
(which only returns a page to the free list when its count reaches zero). `copyout` runs the same check, so kernel writes to user buffers (`read`, `wait`, `pipe`) never modify a shared page.
Build with `-DCOW_FORK=0` to get the old eager copy back for comparison.

## Demand paging and swapping (`kernel/swap.c`)
`growproc` just raises `p->sz`. A load/store page fault (scause 13/15) in `[UBASE, sz)` with no mapping calls `vmfault()`: it allocates a zeroed frame and maps it (`lazy_alloc`). `walkaddr`/`copyout` call the same function, so
kernel-side accesses to untouched pages also work. With `vmctl(VM_SETLIMIT, n)` every lazily allocated page is entered in a resident list (owner page table + va); when the list is full `evict_one()` picks a victim by policy
(FIFO = oldest arrival; Clock = second chance using PTE_A with a rotating hand; LRU = per-page 8-bit age shifted right at each decision with PTE_A shifted in at the top; Random), rewrites the PTE as *invalid + PTE_SWAP + slot number*
(keeping the permission/COW bits), flushes the TLB, writes the 4 KiB to `SWAPSTART + slot*4` blocks (marking the slot busy until the write finishes) and frees the frame. A later fault calls `swap_in()`.
Only pages with refcount 1 are evictable; `fork` brings swapped pages back first and disables eviction while sharing; `uvmunmap` releases swap slots and **flushes the TLB** (stale entries would otherwise let a process write into a frame that was already freed).

## Kernel superpages and the TLB
`kvminit` maps the first 2 MiB of DRAM (kernel image) with 4 KiB pages (text R+X, rest R+W) and the remaining 126 MiB with level-1 leaf PTEs. Every process page table shares root entry 2, so the superpage mappings are shared too.
The emulator keeps 4 KiB translations in a direct-mapped TLB (`--tlb-entries`) and level-1/2 leaves in a small fully-associative superpage TLB (`--stlb-entries`).

## Scheduler switch
`sched_mlfq` is a run-time variable. Under round-robin every slice is 1 tick and priorities never change. Under MLFQ a process that uses its slice drops a level (slices 1/2/4 ticks), sleepers keep their level, all levels are reset every 50 ticks,
and the timer handler preempts the running process as soon as a higher-priority process is runnable.

## Pipes (`kernel/pipe.c`)
A pipe is one kernel page holding a 512-byte ring buffer plus read/write counters and open flags, referenced by two `struct file`s of type `FD_PIPE`.
`pipewrite` sleeps on `&nwrite` when full and wakes readers; `piperead` sleeps on `&nread` when empty while a writer exists, returns 0 (EOF) once all writers are closed;
`pipewrite` returns -1 when no reader remains. The shell builds an N-stage pipeline with N-1 `pipe()` calls, then in each child closes fd 0 / fd 1 and `dup`s the right pipe end into the lowest free descriptor
before closing every pipe fd and calling `exec`.

## File system (`kernel/fs.c`, `xtask/src/mkfs.rs`)
`[boot][super][inodes (64 B each)][bitmap][data]`, 1 KiB blocks, 200 inodes, 12 direct + 1 indirect block per inode (max ≈ 268 KiB per file),
16-byte directory entries (14-char names). There is no journal. `mkfs.rs` and `kernel.h` must agree on these constants.

## Cycle-by-cycle pipeline model (`perf/cycle.rs`)
State: one `Option<Slot>` latch per stage plus a fetch queue. Each `tick()`: (1) every occupied stage does one cycle of work (`rem -= 1`); (2) hand-offs are processed from WB back to IF so a stage freed this cycle can be refilled in the same cycle
(WB retires; MEM->WB marks loads' data ready; EX->MEM marks ALU results ready and resolves branches/JALR, un-blocking fetch; ID->EX resolves JAL, then checks operand readiness against the producers still in MEM/WB (EX must be empty for ID to advance));
(3) fetch pulls the next retired instruction into IF unless a redirect (mispredicted branch, JAL, JALR) is outstanding. `cycles = ticks - 1` because the first tick only makes the initial fetch decision.
Hazard rule: an instruction may enter EX only if the youngest older producer of each source register is `ready`; loads become ready when they finish MEM, everything else when it finishes EX; store data is exempt (needed in MEM).

## Offline page-replacement simulators (`xtask/src/paging.rs`)
`opt` computes next-use indices backwards over the reference string, then evicts the resident page with the largest next-use index on each fault. `fifo`, `clock`, `aging`, `random` replicate kernel/swap.c exactly (arrival-ordered resident list, rotating hand with ordered removal,
8-bit age registers fed from reference bits, the kernel's xorshift32 state carried across runs), so equal fault counts mean the kernel does what the textbook algorithm says.

## Analyzer pipeline
`Cpu::step` fills an `Ev` record (pc, instruction, fetch/data physical addresses, next pc). `perf::Perf::record` classifies it, drives the L1I/L1D/L2 models,
trains all predictors, and charges stall cycles in an analytical 5-stage model (load-use 1, jal 1, jalr 2, mispredict 2, mul +2, div +19, L2 hit 12, memory 100).
In `--sweep` mode the same address stream also feeds banks of extra caches and predictors.
