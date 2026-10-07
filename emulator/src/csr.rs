//! CSR file, privilege checks, trap entry/return.

use crate::cpu::Cpu;
use crate::trap::*;

pub const MSTATUS_SIE: u64 = 1 << 1;
pub const MSTATUS_MIE: u64 = 1 << 3;
pub const MSTATUS_SPIE: u64 = 1 << 5;
pub const MSTATUS_MPIE: u64 = 1 << 7;
pub const MSTATUS_SPP: u64 = 1 << 8;
pub const MSTATUS_MPP: u64 = 3 << 11;
pub const MSTATUS_MPRV: u64 = 1 << 17;
pub const MSTATUS_SUM: u64 = 1 << 18;
pub const MSTATUS_MXR: u64 = 1 << 19;
pub const MSTATUS_TVM: u64 = 1 << 20;
pub const MSTATUS_TW: u64 = 1 << 21;
pub const MSTATUS_TSR: u64 = 1 << 22;
const UXL_SXL: u64 = (2 << 32) | (2 << 34);

const MSTATUS_WMASK: u64 = MSTATUS_SIE | MSTATUS_MIE | MSTATUS_SPIE | MSTATUS_MPIE | MSTATUS_SPP | MSTATUS_MPP
    | MSTATUS_MPRV | MSTATUS_SUM | MSTATUS_MXR | MSTATUS_TVM | MSTATUS_TW | MSTATUS_TSR;
const SSTATUS_WMASK: u64 = MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_SUM | MSTATUS_MXR;
const SSTATUS_RMASK: u64 = SSTATUS_WMASK | (3 << 32);

pub const SSIP: u64 = 1 << 1;
pub const MSIP: u64 = 1 << 3;
pub const STIP: u64 = 1 << 5;
pub const MTIP: u64 = 1 << 7;
pub const SEIP: u64 = 1 << 9;
pub const MEIP: u64 = 1 << 11;
const S_INTS: u64 = SSIP | STIP | SEIP;

pub struct Csr {
    pub mstatus: u64,
    pub medeleg: u64,
    pub mideleg: u64,
    pub mie: u64,
    pub mip_sw: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mcounteren: u64,
    pub scounteren: u64,
    pub stvec: u64,
    pub sscratch: u64,
    pub sepc: u64,
    pub scause: u64,
    pub stval: u64,
    pub satp: u64,
    pub pmpcfg: [u64; 2],
    pub pmpaddr: [u64; 16],
    pub mcycle: u64,
    pub minstret: u64,
}

impl Csr {
    pub fn new() -> Self {
        Csr {
            mstatus: UXL_SXL, medeleg: 0, mideleg: 0, mie: 0, mip_sw: 0, mtvec: 0, mscratch: 0, mepc: 0,
            mcause: 0, mtval: 0, mcounteren: 0, scounteren: 0, stvec: 0, sscratch: 0, sepc: 0, scause: 0,
            stval: 0, satp: 0, pmpcfg: [0; 2], pmpaddr: [0; 16], mcycle: 0, minstret: 0,
        }
    }
}

const MISA: u64 = (2 << 62) | 1 | (1 << 2) | (1 << 8) | (1 << 12) | (1 << 18) | (1 << 20);

fn illegal(addr: u16) -> Trap {
    trap(ILLEGAL, addr as u64) // refined by caller to the instruction bits
}

impl Cpu {
    /// Effective mip: software-writable bits OR'd with hardware lines.
    pub fn mip_eff(&self) -> u64 {
        let mut v = self.csr.mip_sw & S_INTS;
        if self.bus.clint.msip & 1 != 0 {
            v |= MSIP;
        }
        if self.bus.clint.timer_pending() {
            v |= MTIP;
        }
        if self.bus.plic.meip() {
            v |= MEIP;
        }
        if self.bus.plic.seip() {
            v |= SEIP;
        }
        v
    }

    pub fn pending_interrupt(&self) -> Option<u64> {
        let pend = self.mip_eff() & self.csr.mie;
        if pend == 0 {
            return None;
        }
        let m_en = self.priv_ < 3 || self.csr.mstatus & MSTATUS_MIE != 0;
        let s_en = self.priv_ < 1 || (self.priv_ == 1 && self.csr.mstatus & MSTATUS_SIE != 0);
        let mut cand = 0;
        if m_en {
            cand |= pend & !self.csr.mideleg;
        }
        if s_en {
            cand |= pend & self.csr.mideleg;
        }
        for &c in &[11u64, 3, 7, 9, 1, 5] {
            if cand >> c & 1 != 0 {
                return Some(c);
            }
        }
        None
    }

    pub fn take_trap(&mut self, cause: u64, tval: u64, interrupt: bool) {
        let deleg = self.priv_ <= 1
            && if interrupt { self.csr.mideleg >> cause & 1 != 0 } else { self.csr.medeleg >> cause & 1 != 0 };
        let cval = if interrupt { cause | (1 << 63) } else { cause };
        self.reservation = None;
        self.wfi = false;
        let vec_target = |tvec: u64| -> u64 {
            let base = tvec & !3;
            if interrupt && tvec & 1 == 1 { base + 4 * cause } else { base }
        };
        if deleg {
            self.csr.sepc = self.pc & !1;
            self.csr.scause = cval;
            self.csr.stval = tval;
            let s = self.csr.mstatus;
            let mut n = s & !(MSTATUS_SPIE | MSTATUS_SIE | MSTATUS_SPP);
            if s & MSTATUS_SIE != 0 {
                n |= MSTATUS_SPIE;
            }
            if self.priv_ == 1 {
                n |= MSTATUS_SPP;
            }
            self.csr.mstatus = n;
            self.priv_ = 1;
            self.pc = vec_target(self.csr.stvec);
        } else {
            self.csr.mepc = self.pc & !1;
            self.csr.mcause = cval;
            self.csr.mtval = tval;
            let s = self.csr.mstatus;
            let mut n = s & !(MSTATUS_MPIE | MSTATUS_MIE | MSTATUS_MPP);
            if s & MSTATUS_MIE != 0 {
                n |= MSTATUS_MPIE;
            }
            n |= (self.priv_ as u64) << 11;
            self.csr.mstatus = n;
            self.priv_ = 3;
            self.pc = vec_target(self.csr.mtvec);
        }
    }

    pub fn do_mret(&mut self) {
        let s = self.csr.mstatus;
        let mpp = ((s >> 11) & 3) as u8;
        let mut n = s & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
        if s & MSTATUS_MPIE != 0 {
            n |= MSTATUS_MIE;
        }
        n |= MSTATUS_MPIE;
        if mpp != 3 {
            n &= !MSTATUS_MPRV;
        }
        self.csr.mstatus = n;
        self.priv_ = mpp;
        self.next_pc = self.csr.mepc;
        self.reservation = None;
    }

    pub fn do_sret(&mut self) {
        let s = self.csr.mstatus;
        let spp = if s & MSTATUS_SPP != 0 { 1 } else { 0 };
        let mut n = s & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP);
        if s & MSTATUS_SPIE != 0 {
            n |= MSTATUS_SIE;
        }
        n |= MSTATUS_SPIE;
        n &= !MSTATUS_MPRV;
        self.csr.mstatus = n;
        self.priv_ = spp;
        self.next_pc = self.csr.sepc;
        self.reservation = None;
    }

    pub fn csr_read(&mut self, addr: u16) -> Res<u64> {
        let min = ((addr >> 8) & 3) as u8;
        if self.priv_ < min {
            return Err(illegal(addr));
        }
        let c = &self.csr;
        Ok(match addr {
            0x100 => c.mstatus & SSTATUS_RMASK,
            0x104 => c.mie & c.mideleg,
            0x105 => c.stvec,
            0x106 => c.scounteren,
            0x10a => 0,
            0x140 => c.sscratch,
            0x141 => c.sepc,
            0x142 => c.scause,
            0x143 => c.stval,
            0x144 => self.mip_eff() & c.mideleg & S_INTS,
            0x180 => {
                if self.priv_ == 1 && c.mstatus & MSTATUS_TVM != 0 {
                    return Err(illegal(addr));
                }
                c.satp
            }
            0x300 => c.mstatus,
            0x301 => MISA,
            0x302 => c.medeleg,
            0x303 => c.mideleg,
            0x304 => c.mie,
            0x305 => c.mtvec,
            0x306 => c.mcounteren,
            0x30a => 0,
            0x320 => 0,
            0x340 => c.mscratch,
            0x341 => c.mepc,
            0x342 => c.mcause,
            0x343 => c.mtval,
            0x344 => self.mip_eff(),
            0x3a0 => c.pmpcfg[0],
            0x3a2 => c.pmpcfg[1],
            0x3b0..=0x3bf => c.pmpaddr[(addr - 0x3b0) as usize],
            0xb00 => c.mcycle,
            0xb02 => c.minstret,
            0xc00 | 0xc01 | 0xc02 => {
                let bit = (addr - 0xc00) as u64;
                if self.priv_ < 3 && c.mcounteren >> bit & 1 == 0 {
                    return Err(illegal(addr));
                }
                if self.priv_ < 1 && c.scounteren >> bit & 1 == 0 {
                    return Err(illegal(addr));
                }
                match addr {
                    0xc00 => c.mcycle,
                    0xc01 => self.bus.clint.mtime,
                    _ => c.minstret,
                }
            }
            0xb03..=0xb1f | 0x323..=0x33f | 0xc03..=0xc1f => 0, // hpm counters: hardwired zero
            0xf11 | 0xf12 | 0xf13 => 0,
            0xf14 => 0,
            _ => return Err(illegal(addr)),
        })
    }

    pub fn csr_write(&mut self, addr: u16, v: u64) -> Res<()> {
        let min = ((addr >> 8) & 3) as u8;
        if self.priv_ < min || (addr >> 10) & 3 == 3 {
            return Err(illegal(addr));
        }
        match addr {
            0x100 => {
                let c = &mut self.csr;
                c.mstatus = (c.mstatus & !SSTATUS_WMASK) | (v & SSTATUS_WMASK);
            }
            0x104 => {
                let m = self.csr.mideleg & S_INTS;
                self.csr.mie = (self.csr.mie & !m) | (v & m);
            }
            0x105 => self.csr.stvec = v & !2,
            0x106 => self.csr.scounteren = v & 7,
            0x10a => {}
            0x140 => self.csr.sscratch = v,
            0x141 => self.csr.sepc = v & !1,
            0x142 => self.csr.scause = v,
            0x143 => self.csr.stval = v,
            0x144 => {
                if self.csr.mideleg & SSIP != 0 {
                    self.csr.mip_sw = (self.csr.mip_sw & !SSIP) | (v & SSIP);
                }
            }
            0x180 => {
                if self.priv_ == 1 && self.csr.mstatus & MSTATUS_TVM != 0 {
                    return Err(illegal(addr));
                }
                let mode = v >> 60;
                if mode == 0 || mode == 8 {
                    self.csr.satp = v & !(0xfu64 << 60) | (mode << 60);
                    self.tlb.flush();
                }
            }
            0x300 => {
                let c = &mut self.csr;
                let mut n = (c.mstatus & !MSTATUS_WMASK) | (v & MSTATUS_WMASK);
                if (n >> 11) & 3 == 2 {
                    n &= !MSTATUS_MPP; // reserved MPP value: legalise to U
                }
                c.mstatus = n;
            }
            0x301 => {}
            0x302 => self.csr.medeleg = v & 0xb3ff,
            0x303 => self.csr.mideleg = v & S_INTS,
            0x304 => self.csr.mie = v & 0xaaa,
            0x305 => self.csr.mtvec = v & !2,
            0x306 => self.csr.mcounteren = v & 7,
            0x30a | 0x320 => {}
            0x340 => self.csr.mscratch = v,
            0x341 => self.csr.mepc = v & !1,
            0x342 => self.csr.mcause = v,
            0x343 => self.csr.mtval = v,
            0x344 => self.csr.mip_sw = v & S_INTS,
            0x3a0 => self.csr.pmpcfg[0] = v,
            0x3a2 => self.csr.pmpcfg[1] = v,
            0x3b0..=0x3bf => self.csr.pmpaddr[(addr - 0x3b0) as usize] = v & ((1 << 54) - 1),
            0xb00 => {
                self.csr.mcycle = v;
                self.skip_cycle = true;
            }
            0xb02 => {
                self.csr.minstret = v;
                self.skip_instret = true;
            }
            0xb03..=0xb1f | 0x323..=0x33f => {}
            _ => return Err(illegal(addr)),
        }
        Ok(())
    }
}
