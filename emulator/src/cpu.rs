//! RV64IMAC hart: fetch / decode / execute, with M/S/U privilege, Sv39 MMU,
//! traps, interrupts and an optional microarchitecture analyzer hook.

use crate::bus::Bus;
use crate::csr::*;
use crate::mmu::{Acc, Tlb};
use crate::perf::{Ev, Perf};
use crate::rvc::expand;
use crate::trap::*;
use std::io::Write;

/// Instructions per `mtime` tick (timebase = 1/TIME_DIV of the instruction clock).
pub const TIME_DIV: u64 = 10;

pub struct Cpu {
    pub regs: [u64; 32],
    pub pc: u64,
    pub next_pc: u64,
    pub priv_: u8,
    pub csr: Csr,
    pub bus: Bus,
    pub tlb: Tlb,
    pub reservation: Option<u64>,
    pub wfi: bool,
    pub ev: Ev,
    pub perf: Option<Box<Perf>>,
    pub trace: Option<std::io::BufWriter<std::fs::File>>,
    pub idle_exit: bool,
    pub skip_instret: bool,
    pub skip_cycle: bool,
    steps: u64,
    tick_acc: u64,
}

#[inline]
fn imm_i(i: u32) -> u64 { ((i as i32) >> 20) as i64 as u64 }
#[inline]
fn imm_s(i: u32) -> u64 { ((((i as i32) >> 25) << 5) as i64 as u64) | ((i >> 7) & 0x1f) as u64 }
#[inline]
fn imm_b(i: u32) -> u64 {
    ((((i as i32) >> 31) << 12) as i64 as u64) | (((i >> 7) & 1) << 11) as u64 | (((i >> 25) & 0x3f) << 5) as u64 | (((i >> 8) & 0xf) << 1) as u64
}
#[inline]
fn imm_u(i: u32) -> u64 { (i & 0xffff_f000) as i32 as i64 as u64 }
#[inline]
fn imm_j(i: u32) -> u64 {
    ((((i as i32) >> 31) << 20) as i64 as u64) | (i & 0xff000) as u64 | (((i >> 20) & 1) << 11) as u64 | (((i >> 21) & 0x3ff) << 1) as u64
}
#[inline]
fn sext32(v: u32) -> u64 { v as i32 as i64 as u64 }

impl Cpu {
    pub fn new(bus: Bus, entry: u64) -> Self {
        Cpu {
            regs: [0; 32], pc: entry, next_pc: entry, priv_: 3, csr: Csr::new(), bus, tlb: Tlb::new(crate::mmu::DEFAULT_TLB_ENTRIES, crate::mmu::DEFAULT_STLB_ENTRIES),
            reservation: None, wfi: false, ev: Ev::default(), perf: None, trace: None, idle_exit: false,
            skip_instret: false, skip_cycle: false,
            steps: 0, tick_acc: 0,
        }
    }

    #[inline]
    fn wr(&mut self, rd: usize, v: u64) {
        if rd != 0 {
            self.regs[rd] = v;
        }
    }

    /// Execute one step. Returns false once the guest has powered off (or deadlocked).
    pub fn step(&mut self) -> bool {
        if self.bus.exit.is_some() || self.idle_exit {
            return false;
        }
        self.steps += 1;
        self.tick_acc += 1;
        if self.tick_acc == TIME_DIV {
            self.tick_acc = 0;
            self.bus.clint.mtime = self.bus.clint.mtime.wrapping_add(1);
        }
        if self.steps & 0xff == 0 {
            self.bus.poll();
        }
        if self.wfi {
            if self.mip_eff() & self.csr.mie == 0 {
                return self.idle();
            }
            self.wfi = false;
        }
        if let Some(c) = self.pending_interrupt() {
            self.take_trap(c, 0, true);
            return true;
        }
        self.ev.mem_kind = 0;
        match self.fetch_exec() {
            Ok(()) => {
                if !self.skip_instret {
                    self.csr.minstret = self.csr.minstret.wrapping_add(1);
                }
                if !self.skip_cycle {
                    self.csr.mcycle = self.csr.mcycle.wrapping_add(1);
                }
                self.skip_instret = false;
                self.skip_cycle = false;
                if self.perf.is_some() || self.trace.is_some() {
                    self.ev.pc = self.pc;
                    self.ev.next_pc = self.next_pc;
                    if let Some(p) = self.perf.as_deref_mut() {
                        p.record(&self.ev);
                    }
                    if let Some(t) = self.trace.as_mut() {
                        let _ = if self.ev.mem_kind != 0 {
                            writeln!(t, "{:x} {:08x} {} {:x}", self.ev.pc, self.ev.inst, if self.ev.mem_kind == 2 { 'S' } else { 'L' }, self.ev.mem_pa)
                        } else {
                            writeln!(t, "{:x} {:08x}", self.ev.pc, self.ev.inst)
                        };
                    }
                }
                self.pc = self.next_pc;
            }
            Err(t) => self.take_trap(t.cause, t.tval, false),
        }
        true
    }

    /// WFI with nothing pending: fast-forward virtual time to the next event.
    fn idle(&mut self) -> bool {
        self.bus.poll();
        if self.mip_eff() & self.csr.mie != 0 {
            return true;
        }
        let c = &mut self.bus.clint;
        if c.mtimecmp != u64::MAX && c.mtime < c.mtimecmp {
            if self.bus.uart.has_live_input {
                // keep real-time pacing so the host CPU is not pegged while idle
                std::thread::sleep(std::time::Duration::from_micros(500));
                c.mtime = c.mtime.saturating_add(50).min(c.mtimecmp);
            } else {
                c.mtime = c.mtimecmp;
            }
            return true;
        }
        if self.bus.uart.has_live_input {
            std::thread::sleep(std::time::Duration::from_millis(1));
            return true;
        }
        if self.bus.uart.rx_ready() {
            return true;
        }
        eprintln!("[rvsim] guest is in WFI with no possible wake-up source; stopping.");
        self.idle_exit = true;
        false
    }

    fn fetch_exec(&mut self) -> Res<()> {
        let pc = self.pc;
        let pa = self.translate(pc, Acc::Fetch)?;
        self.ev.fetch_pa = pa;
        let lo = match self.bus.load(pa, 2) {
            Some(v) => v as u32,
            None => return Err(trap(INSN_ACCESS, pc)),
        };
        let (inst, len, raw) = if lo & 3 != 3 {
            match expand(lo as u16) {
                Some(i) => (i, 2u64, lo as u64),
                None => return Err(trap(ILLEGAL, lo as u64)),
            }
        } else {
            let pc2 = pc.wrapping_add(2);
            let pa2 = if pc2 & 0xfff == 0 { self.translate(pc2, Acc::Fetch)? } else { pa + 2 };
            let hi = match self.bus.load(pa2, 2) {
                Some(v) => v as u32,
                None => return Err(trap(INSN_ACCESS, pc2)),
            };
            let i = lo | (hi << 16);
            (i, 4u64, i as u64)
        };
        self.ev.inst = inst;
        self.ev.len = len as u8;
        self.next_pc = pc.wrapping_add(len);
        self.exec(inst, len, raw)
    }

    fn exec(&mut self, inst: u32, len: u64, raw: u64) -> Res<()> {
        let opcode = inst & 0x7f;
        let rd = ((inst >> 7) & 0x1f) as usize;
        let f3 = (inst >> 12) & 7;
        let rs1 = ((inst >> 15) & 0x1f) as usize;
        let rs2 = ((inst >> 20) & 0x1f) as usize;
        let f7 = inst >> 25;
        let pc = self.pc;
        let a = self.regs[rs1];
        let b = self.regs[rs2];
        let ill = trap(ILLEGAL, raw);

        match opcode {
            0x37 => self.wr(rd, imm_u(inst)),
            0x17 => self.wr(rd, pc.wrapping_add(imm_u(inst))),
            0x6f => {
                self.wr(rd, pc.wrapping_add(len));
                self.next_pc = pc.wrapping_add(imm_j(inst));
            }
            0x67 => {
                if f3 != 0 {
                    return Err(ill);
                }
                let t = a.wrapping_add(imm_i(inst)) & !1;
                self.wr(rd, pc.wrapping_add(len));
                self.next_pc = t;
            }
            0x63 => {
                let taken = match f3 {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i64) < (b as i64),
                    5 => (a as i64) >= (b as i64),
                    6 => a < b,
                    7 => a >= b,
                    _ => return Err(ill),
                };
                if taken {
                    self.next_pc = pc.wrapping_add(imm_b(inst));
                }
            }
            0x03 => {
                let addr = a.wrapping_add(imm_i(inst));
                let v = match f3 {
                    0 => self.mem_read(addr, 1, Acc::Load)? as i8 as i64 as u64,
                    1 => self.mem_read(addr, 2, Acc::Load)? as i16 as i64 as u64,
                    2 => self.mem_read(addr, 4, Acc::Load)? as i32 as i64 as u64,
                    3 => self.mem_read(addr, 8, Acc::Load)?,
                    4 => self.mem_read(addr, 1, Acc::Load)?,
                    5 => self.mem_read(addr, 2, Acc::Load)?,
                    6 => self.mem_read(addr, 4, Acc::Load)?,
                    _ => return Err(ill),
                };
                self.wr(rd, v);
            }
            0x23 => {
                let addr = a.wrapping_add(imm_s(inst));
                if f3 > 3 {
                    return Err(ill);
                }
                self.mem_write(addr, 1 << f3, b)?;
            }
            0x13 => {
                let imm = imm_i(inst);
                let v = match f3 {
                    0 => a.wrapping_add(imm),
                    1 => {
                        if inst >> 26 != 0 {
                            return Err(ill);
                        }
                        a << (imm & 63)
                    }
                    2 => ((a as i64) < (imm as i64)) as u64,
                    3 => (a < imm) as u64,
                    4 => a ^ imm,
                    5 => match inst >> 26 {
                        0x00 => a >> (imm & 63),
                        0x10 => ((a as i64) >> (imm & 63)) as u64,
                        _ => return Err(ill),
                    },
                    6 => a | imm,
                    _ => a & imm,
                };
                self.wr(rd, v);
            }
            0x1b => {
                let imm = imm_i(inst);
                let sh = rs2 as u32;
                let v = match f3 {
                    0 => sext32(a.wrapping_add(imm) as u32),
                    1 if f7 == 0 => sext32((a as u32) << sh),
                    5 if f7 == 0 => sext32((a as u32) >> sh),
                    5 if f7 == 0x20 => ((a as i32) >> sh) as i64 as u64,
                    _ => return Err(ill),
                };
                self.wr(rd, v);
            }
            0x33 => {
                let v = if f7 == 1 {
                    match f3 {
                        0 => a.wrapping_mul(b),
                        1 => (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64,
                        2 => (((a as i64 as i128) * (b as u128 as i128)) >> 64) as u64,
                        3 => (((a as u128) * (b as u128)) >> 64) as u64,
                        4 => {
                            if b == 0 { u64::MAX } else if a == i64::MIN as u64 && b == u64::MAX { a } else { ((a as i64) / (b as i64)) as u64 }
                        }
                        5 => if b == 0 { u64::MAX } else { a / b },
                        6 => {
                            if b == 0 { a } else if a == i64::MIN as u64 && b == u64::MAX { 0 } else { ((a as i64) % (b as i64)) as u64 }
                        }
                        _ => if b == 0 { a } else { a % b },
                    }
                } else {
                    match (f7, f3) {
                        (0x00, 0) => a.wrapping_add(b),
                        (0x20, 0) => a.wrapping_sub(b),
                        (0x00, 1) => a << (b & 63),
                        (0x00, 2) => ((a as i64) < (b as i64)) as u64,
                        (0x00, 3) => (a < b) as u64,
                        (0x00, 4) => a ^ b,
                        (0x00, 5) => a >> (b & 63),
                        (0x20, 5) => ((a as i64) >> (b & 63)) as u64,
                        (0x00, 6) => a | b,
                        (0x00, 7) => a & b,
                        _ => return Err(ill),
                    }
                };
                self.wr(rd, v);
            }
            0x3b => {
                let (a32, b32) = (a as u32, b as u32);
                let v = if f7 == 1 {
                    match f3 {
                        0 => sext32(a32.wrapping_mul(b32)),
                        4 => {
                            let (x, y) = (a32 as i32, b32 as i32);
                            if y == 0 { u64::MAX } else if x == i32::MIN && y == -1 { x as i64 as u64 } else { (x / y) as i64 as u64 }
                        }
                        5 => if b32 == 0 { u64::MAX } else { sext32(a32 / b32) },
                        6 => {
                            let (x, y) = (a32 as i32, b32 as i32);
                            if y == 0 { x as i64 as u64 } else if x == i32::MIN && y == -1 { 0 } else { (x % y) as i64 as u64 }
                        }
                        7 => if b32 == 0 { sext32(a32) } else { sext32(a32 % b32) },
                        _ => return Err(ill),
                    }
                } else {
                    match (f7, f3) {
                        (0x00, 0) => sext32(a32.wrapping_add(b32)),
                        (0x20, 0) => sext32(a32.wrapping_sub(b32)),
                        (0x00, 1) => sext32(a32 << (b32 & 31)),
                        (0x00, 5) => sext32(a32 >> (b32 & 31)),
                        (0x20, 5) => ((a32 as i32) >> (b32 & 31)) as i64 as u64,
                        _ => return Err(ill),
                    }
                };
                self.wr(rd, v);
            }
            0x0f => {} // FENCE / FENCE.I: no-ops (no I-cache modelled in the interpreter)
            0x2f => self.exec_amo(inst, rd, f3, a, b, rs2)?,
            0x73 => self.exec_system(inst, raw, rd, f3, rs1, a)?,
            _ => return Err(ill),
        }
        Ok(())
    }

    fn exec_amo(&mut self, inst: u32, rd: usize, f3: u32, a: u64, b: u64, rs2: usize) -> Res<()> {
        let ill = trap(ILLEGAL, inst as u64);
        let size: usize = match f3 {
            2 => 4,
            3 => 8,
            _ => return Err(ill),
        };
        let f5 = inst >> 27;
        let addr = a;
        let fix = |v: u64| if size == 4 { v as i32 as i64 as u64 } else { v };
        match f5 {
            0x02 => {
                if rs2 != 0 {
                    return Err(ill);
                }
                if addr % size as u64 != 0 {
                    return Err(trap(LOAD_MISALIGNED, addr));
                }
                let v = fix(self.mem_read(addr, size, Acc::Load)?);
                self.reservation = Some(addr);
                self.wr(rd, v);
            }
            0x03 => {
                if addr % size as u64 != 0 {
                    return Err(trap(STORE_MISALIGNED, addr));
                }
                let ok = self.reservation == Some(addr);
                self.reservation = None;
                if ok {
                    self.mem_write(addr, size, b)?;
                    self.wr(rd, 0);
                } else {
                    self.wr(rd, 1);
                }
            }
            0x00 | 0x01 | 0x04 | 0x08 | 0x0c | 0x10 | 0x14 | 0x18 | 0x1c => {
                if addr % size as u64 != 0 {
                    return Err(trap(STORE_MISALIGNED, addr));
                }
                let old = fix(self.mem_read(addr, size, Acc::Store)?);
                let (x, y) = if size == 4 { (old as i32 as i64, b as i32 as i64) } else { (old as i64, b as i64) };
                let (ux, uy) = if size == 4 { (old & 0xffff_ffff, b & 0xffff_ffff) } else { (old, b) };
                let new = match f5 {
                    0x00 => old.wrapping_add(b),
                    0x01 => b,
                    0x04 => old ^ b,
                    0x08 => old | b,
                    0x0c => old & b,
                    0x10 => if x < y { old } else { b },
                    0x14 => if x > y { old } else { b },
                    0x18 => if ux < uy { old } else { b },
                    _ => if ux > uy { old } else { b },
                };
                self.ev.mem_kind = 2;
                self.mem_write(addr, size, new)?;
                self.wr(rd, old);
            }
            _ => return Err(ill),
        }
        Ok(())
    }

    fn exec_system(&mut self, inst: u32, raw: u64, rd: usize, f3: u32, rs1: usize, a: u64) -> Res<()> {
        let ill = trap(ILLEGAL, raw);
        if f3 == 0 {
            match inst {
                0x0000_0073 => return Err(trap(ECALL_U + self.priv_ as u64, 0)),
                0x0010_0073 => return Err(trap(BREAKPOINT, self.pc)),
                0x3020_0073 => {
                    if self.priv_ < 3 {
                        return Err(ill);
                    }
                    self.do_mret();
                }
                0x1020_0073 => {
                    if self.priv_ < 1 || (self.priv_ == 1 && self.csr.mstatus & MSTATUS_TSR != 0) {
                        return Err(ill);
                    }
                    self.do_sret();
                }
                0x1050_0073 => {
                    if self.priv_ == 0 || (self.priv_ == 1 && self.csr.mstatus & MSTATUS_TW != 0) {
                        return Err(ill);
                    }
                    self.wfi = true;
                }
                _ => {
                    if inst >> 25 == 0x09 && rd == 0 {
                        if self.priv_ == 0 || (self.priv_ == 1 && self.csr.mstatus & MSTATUS_TVM != 0) {
                            return Err(ill);
                        }
                        self.tlb.flush();
                    } else {
                        return Err(ill);
                    }
                }
            }
            return Ok(());
        }
        if f3 == 4 {
            return Err(ill);
        }
        let addr = (inst >> 20) as u16;
        let src = if f3 & 4 != 0 { rs1 as u64 } else { a };
        let op = f3 & 3;
        let write = op == 1 || rs1 != 0;
        let fix = |t: Trap| trap(t.cause, raw);
        let old = if op == 1 && rd == 0 { 0 } else { self.csr_read(addr).map_err(fix)? };
        if write {
            let new = match op {
                1 => src,
                2 => old | src,
                _ => old & !src,
            };
            self.csr_write(addr, new).map_err(fix)?;
        }
        self.wr(rd, old);
        Ok(())
    }
}
