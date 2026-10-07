//! Sv39 address translation with a direct-mapped TLB, plus the guest memory
//! access helpers (alignment-agnostic, page-crossing aware).

use crate::cpu::Cpu;
use crate::trap::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Acc {
    Fetch,
    Load,
    Store,
}

pub const DEFAULT_TLB_ENTRIES: usize = 256;
pub const DEFAULT_STLB_ENTRIES: usize = 16;

#[derive(Clone, Copy, Default)]
struct TlbEnt {
    valid: bool,
    asid: u16,
    vpn: u64,
    ppn: u64,
    r: bool,
    w: bool,
    x: bool,
    u: bool,
    d: bool,
}

/// Entry of the small fully-associative TLB that holds 2 MiB / 1 GiB leaf mappings.
#[derive(Clone, Copy, Default)]
struct SuperEnt {
    valid: bool,
    asid: u16,
    level: u32,
    tag: u64,      // vpn >> (9 * level)
    ppn_base: u64, // leaf ppn with the low 9*level bits zero
    r: bool,
    w: bool,
    x: bool,
    u: bool,
    d: bool,
}

/// Two-level-page-size TLB: a direct-mapped 4 KiB TLB plus a tiny fully-associative superpage TLB.
pub struct Tlb {
    ent: Vec<TlbEnt>,
    mask: usize,
    sup: Vec<SuperEnt>,
    sup_next: usize,
    pub entries: usize,
    pub sup_entries: usize,
    pub hits: u64,
    pub sup_hits: u64,
    pub misses: u64,
    pub flushes: u64,
}

impl Tlb {
    pub fn new(entries: usize, sup_entries: usize) -> Self {
        let entries = entries.max(1).next_power_of_two();
        Tlb {
            ent: vec![TlbEnt::default(); entries],
            mask: entries - 1,
            sup: vec![SuperEnt::default(); sup_entries.max(1)],
            sup_next: 0,
            entries,
            sup_entries: sup_entries.max(1),
            hits: 0,
            sup_hits: 0,
            misses: 0,
            flushes: 0,
        }
    }
    pub fn flush(&mut self) {
        for e in self.ent.iter_mut() {
            e.valid = false;
        }
        for e in self.sup.iter_mut() {
            e.valid = false;
        }
        self.flushes += 1;
    }
}

#[inline]
fn page_fault(acc: Acc, va: u64) -> Trap {
    trap(
        match acc {
            Acc::Fetch => INSN_PAGE,
            Acc::Load => LOAD_PAGE,
            Acc::Store => STORE_PAGE,
        },
        va,
    )
}

#[inline]
fn access_fault(acc: Acc, va: u64) -> Trap {
    trap(
        match acc {
            Acc::Fetch => INSN_ACCESS,
            Acc::Load => LOAD_ACCESS,
            Acc::Store => STORE_ACCESS,
        },
        va,
    )
}

#[inline]
fn perm_ok(r: bool, w: bool, x: bool, u: bool, eff: u8, acc: Acc, sum: bool, mxr: bool) -> bool {
    if u {
        if eff == 1 && (!sum || acc == Acc::Fetch) {
            return false;
        }
    } else if eff == 0 {
        return false;
    }
    match acc {
        Acc::Fetch => x,
        Acc::Load => r || (mxr && x),
        Acc::Store => w,
    }
}

impl Cpu {
    pub fn translate(&mut self, va: u64, acc: Acc) -> Res<u64> {
        let status = self.csr.mstatus;
        let eff = if acc != Acc::Fetch && (status >> 17) & 1 == 1 { ((status >> 11) & 3) as u8 } else { self.priv_ };
        let satp = self.csr.satp;
        if eff == 3 || (satp >> 60) == 0 {
            return Ok(va);
        }
        // Sv39 canonical-address check
        let top = (va as i64) >> 38;
        if top != 0 && top != -1 {
            return Err(page_fault(acc, va));
        }
        let sum = (status >> 18) & 1 == 1;
        let mxr = (status >> 19) & 1 == 1;
        let vpn = va >> 12;
        let asid = ((satp >> 44) & 0xffff) as u16;
        let idx = (vpn as usize) & self.tlb.mask;
        let e = self.tlb.ent[idx];
        if e.valid && e.vpn == vpn && e.asid == asid && (acc != Acc::Store || e.d) {
            if perm_ok(e.r, e.w, e.x, e.u, eff, acc, sum, mxr) {
                self.tlb.hits += 1;
                return Ok((e.ppn << 12) | (va & 0xfff));
            }
            return Err(page_fault(acc, va));
        }
        // superpage TLB (fully associative)
        for k in 0..self.tlb.sup.len() {
            let s = self.tlb.sup[k];
            if s.valid && s.asid == asid && (vpn >> (9 * s.level)) == s.tag && (acc != Acc::Store || s.d) {
                if perm_ok(s.r, s.w, s.x, s.u, eff, acc, sum, mxr) {
                    self.tlb.sup_hits += 1;
                    let low = vpn & ((1u64 << (9 * s.level)) - 1);
                    return Ok(((s.ppn_base | low) << 12) | (va & 0xfff));
                }
                return Err(page_fault(acc, va));
            }
        }
        self.tlb.misses += 1;

        // Page-table walk
        let mut a = (satp & ((1u64 << 44) - 1)) << 12;
        let vpns = [(va >> 12) & 0x1ff, (va >> 21) & 0x1ff, (va >> 30) & 0x1ff];
        let mut level = 2usize;
        let (mut pte, pte_addr);
        loop {
            let addr = a + vpns[level] * 8;
            pte = match self.bus.load(addr, 8) {
                Some(p) => p,
                None => return Err(access_fault(acc, va)),
            };
            if pte & 1 == 0 || (pte & 2 == 0 && pte & 4 != 0) || (pte >> 54) != 0 {
                return Err(page_fault(acc, va));
            }
            if pte & 0xe != 0 {
                pte_addr = addr;
                break;
            }
            if level == 0 {
                return Err(page_fault(acc, va));
            }
            level -= 1;
            a = ((pte >> 10) & ((1u64 << 44) - 1)) << 12;
        }
        let (r, w, x, u) = (pte & 2 != 0, pte & 4 != 0, pte & 8 != 0, pte & 16 != 0);
        if !perm_ok(r, w, x, u, eff, acc, sum, mxr) {
            return Err(page_fault(acc, va));
        }
        let mut ppn = (pte >> 10) & ((1u64 << 44) - 1);
        let ppn_base = ppn;
        if level > 0 {
            let m = (1u64 << (9 * level)) - 1;
            if ppn & m != 0 {
                return Err(page_fault(acc, va)); // misaligned superpage
            }
            ppn |= vpn & m;
        }
        // Hardware A/D update
        let need_d = acc == Acc::Store;
        if pte & 0x40 == 0 || (need_d && pte & 0x80 == 0) {
            pte |= 0x40;
            if need_d {
                pte |= 0x80;
            }
            if !self.bus.store(pte_addr, 8, pte) {
                return Err(access_fault(acc, va));
            }
        }
        if level > 0 {
            let k = self.tlb.sup_next % self.tlb.sup.len();
            self.tlb.sup_next += 1;
            self.tlb.sup[k] = SuperEnt { valid: true, asid, level: level as u32, tag: vpn >> (9 * level), ppn_base, r, w, x, u, d: pte & 0x80 != 0 };
        } else {
            self.tlb.ent[idx] = TlbEnt { valid: true, asid, vpn, ppn, r, w, x, u, d: pte & 0x80 != 0 };
        }
        Ok((ppn << 12) | (va & 0xfff))
    }

    /// Guest data read of `size` bytes (1,2,4,8). `acc` is Load, or Store for AMOs.
    pub fn mem_read(&mut self, va: u64, size: usize, acc: Acc) -> Res<u64> {
        if (va & 0xfff) + size as u64 > 0x1000 {
            let mut v = 0u64;
            for i in 0..size {
                v |= self.mem_read(va.wrapping_add(i as u64), 1, acc)? << (8 * i);
            }
            return Ok(v);
        }
        let pa = self.translate(va, acc)?;
        self.ev.mem_pa = pa;
        self.ev.mem_size = size as u8;
        self.ev.mem_kind = if self.ev.mem_kind == 2 { 2 } else { 1 };
        match self.bus.load(pa, size) {
            Some(v) => Ok(v),
            None => Err(access_fault(acc, va)),
        }
    }

    pub fn mem_write(&mut self, va: u64, size: usize, val: u64) -> Res<()> {
        if (va & 0xfff) + size as u64 > 0x1000 {
            // Translate every byte first so the store is all-or-nothing.
            for i in 0..size {
                self.translate(va.wrapping_add(i as u64), Acc::Store)?;
            }
            for i in 0..size {
                self.mem_write(va.wrapping_add(i as u64), 1, val >> (8 * i))?;
            }
            return Ok(());
        }
        let pa = self.translate(va, Acc::Store)?;
        self.ev.mem_pa = pa;
        self.ev.mem_size = size as u8;
        self.ev.mem_kind = 2;
        if self.bus.store(pa, size, val) {
            Ok(())
        } else {
            Err(access_fault(Acc::Store, va))
        }
    }
}
