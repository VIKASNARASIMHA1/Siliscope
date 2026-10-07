//! Cycle-by-cycle model of a classic 5-stage in-order pipeline (IF ID EX MEM WB).
//!
//! Unlike `pipeline.rs` (which adds up penalty cycles with a formula), this model keeps one
//! latch per stage and advances the whole machine one clock at a time:
//!
//!   * each stage holds at most one instruction; an instruction moves on only when its stage
//!     has finished and the next stage is free (so stalls propagate backwards automatically);
//!   * IF takes 1 + I-cache-miss cycles, MEM takes 1 + D-cache-miss cycles (blocking caches),
//!     EX takes 1 cycle, 3 for a multiply and 20 for a divide;
//!   * full forwarding: an ALU result can be used by the very next instruction; a load result
//!     only after the load has finished MEM (the classic 1-cycle load-use stall); store data
//!     is needed late (MEM), so a load feeding a store's data causes no stall;
//!   * control flow: a conditional branch is resolved in EX (a misprediction squashes the two
//!     younger instructions = 2 bubbles), JAL redirects from ID (1 bubble), JALR from EX (2);
//!     correctly predicted branches cost nothing (perfect BTB).
//!
//! The model is fed the *retired* instruction stream, so wrong-path instructions are
//! represented by holding the fetch unit idle until the redirect happens.

use super::pipeline::{Info, DIV_EXTRA, MUL_EXTRA};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub struct Uop {
    pub info: Info,
    pub istall: u32,
    pub dstall: u32,
    pub mispredict: bool,
}

#[derive(Clone, Copy)]
struct Slot {
    u: Uop,
    rem: u32,    // cycles of work still to do in the current stage
    ready: bool, // result available for forwarding
    seq: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum BlockAt {
    Id,
    Ex,
}

#[derive(Default, Clone, Copy)]
pub struct StallStats {
    pub load_use: u64,       // cycles the instruction in ID waited for an operand
    pub ex_busy: u64,        // cycles ID waited because EX was occupied (mul/div or an earlier stall)
    pub fetch_blocked: u64,  // cycles the fetch unit sat idle waiting for a branch/jump to resolve
    pub imiss: u64,          // cycles IF spent waiting on an I-cache miss
    pub dmiss: u64,          // cycles MEM spent waiting on a D-cache miss
}

pub struct CycleModel {
    ifs: Option<Slot>,
    ids: Option<Slot>,
    exs: Option<Slot>,
    mems: Option<Slot>,
    wbs: Option<Slot>,
    queue: VecDeque<Uop>,
    seq_next: u64,
    block: Option<(u64, BlockAt)>,
    ticks: u64,
    pub retired: u64,
    pub stalls: StallStats,
}

impl CycleModel {
    pub fn new() -> Self {
        CycleModel {
            ifs: None, ids: None, exs: None, mems: None, wbs: None,
            queue: VecDeque::new(), seq_next: 0, block: None, ticks: 0, retired: 0,
            stalls: StallStats::default(),
        }
    }

    /// Total clock cycles so far (the first tick only performs the initial fetch decision).
    pub fn cycles(&self) -> u64 {
        self.ticks.saturating_sub(1)
    }

    pub fn cpi(&self) -> f64 {
        if self.retired == 0 { 0.0 } else { self.cycles() as f64 / self.retired as f64 }
    }

    /// Feed one retired instruction and run the clock until the front end has accepted it.
    pub fn push(&mut self, u: Uop) {
        self.queue.push_back(u);
        while !self.queue.is_empty() {
            self.tick();
        }
    }

    /// Run until every instruction has left the pipeline.
    pub fn drain(&mut self) {
        while !self.queue.is_empty() || self.ifs.is_some() || self.ids.is_some() || self.exs.is_some()
            || self.mems.is_some() || self.wbs.is_some() {
            self.tick();
        }
    }

    fn producer_ready(&self, r: u8) -> bool {
        if r == 0 {
            return true;
        }
        // Only MEM and WB can hold older producers here (EX is empty whenever ID may advance);
        // MEM is younger than WB, so it wins.
        for s in [&self.mems, &self.wbs] {
            if let Some(p) = s {
                if p.u.info.writes_rd && p.u.info.rd == r {
                    return p.ready;
                }
            }
        }
        true
    }

    fn operands_ready(&self, i: &Info) -> bool {
        if i.uses_rs1 && !self.producer_ready(i.rs1) {
            return false;
        }
        let store_only = i.store && !i.load;           // store data is needed in MEM, not EX
        if i.uses_rs2 && !store_only && !self.producer_ready(i.rs2) {
            return false;
        }
        true
    }

    fn tick(&mut self) {
        self.ticks += 1;

        // 1. every occupied stage does one cycle of work
        for s in [&mut self.ifs, &mut self.ids, &mut self.exs, &mut self.mems, &mut self.wbs] {
            if let Some(sl) = s {
                if sl.rem > 0 {
                    sl.rem -= 1;
                }
            }
        }
        if matches!(self.ifs, Some(s) if s.rem > 0) { self.stalls.imiss += 1; }
        if matches!(self.mems, Some(s) if s.rem > 0) { self.stalls.dmiss += 1; }

        // 2. hand-offs, last stage first so a freed stage can be refilled in the same cycle
        // WB: retire
        if let Some(w) = self.wbs {
            if w.rem == 0 {
                self.retired += 1;
                self.wbs = None;
            }
        }
        // MEM -> WB
        if let Some(mut m) = self.mems {
            if m.rem == 0 {
                m.ready = true;                     // loads/AMOs now have their data
                if self.wbs.is_none() {
                    m.rem = 1;
                    self.wbs = Some(m);
                    self.mems = None;
                } else {
                    self.mems = Some(m);
                }
            }
        }
        // EX -> MEM
        if let Some(mut e) = self.exs {
            if e.rem == 0 {
                if !e.u.info.load {
                    e.ready = true;                 // ALU / mul / div / jal results forward from EX
                }
                if let Some((seq, BlockAt::Ex)) = self.block {
                    if seq == e.seq {
                        self.block = None;          // branch / jalr resolved: fetch may redirect
                    }
                }
                if self.mems.is_none() {
                    e.rem = if e.u.info.load || e.u.info.store { 1 + e.u.dstall } else { 1 };
                    self.mems = Some(e);
                    self.exs = None;
                } else {
                    self.exs = Some(e);
                }
            }
        }
        // ID -> EX
        if let Some(d) = self.ids {
            if d.rem == 0 {
                if d.u.info.jal {
                    if let Some((seq, BlockAt::Id)) = self.block {
                        if seq == d.seq {
                            self.block = None;      // jal target known after decode
                        }
                    }
                }
                if self.exs.is_some() {
                    self.stalls.ex_busy += 1;
                } else if !self.operands_ready(&d.u.info) {
                    self.stalls.load_use += 1;
                } else {
                    let mut s = d;
                    let extra = if d.u.info.div { DIV_EXTRA } else if d.u.info.mul { MUL_EXTRA } else { 0 };
                    s.rem = 1 + extra as u32;
                    s.ready = false;
                    self.exs = Some(s);
                    self.ids = None;
                }
            }
        }
        // IF -> ID
        if let Some(f) = self.ifs {
            if f.rem == 0 && self.ids.is_none() {
                let mut s = f;
                s.rem = 1;
                self.ids = Some(s);
                self.ifs = None;
            }
        }
        // fetch the next instruction
        if self.ifs.is_none() {
            if self.block.is_some() {
                self.stalls.fetch_blocked += 1;
            } else if let Some(u) = self.queue.pop_front() {
                let seq = self.seq_next;
                self.seq_next += 1;
                if u.info.cond_branch && u.mispredict {
                    self.block = Some((seq, BlockAt::Ex));
                } else if u.info.jal {
                    self.block = Some((seq, BlockAt::Id));
                } else if u.info.jalr {
                    self.block = Some((seq, BlockAt::Ex));
                }
                self.ifs = Some(Slot { u, rem: 1 + u.istall, ready: false, seq });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::pipeline::classify;

    // tiny encoders for the instruction classes we need
    fn addi(rd: u32, rs1: u32) -> u32 { 0x13 | rd << 7 | rs1 << 15 | 1 << 20 }
    fn add(rd: u32, rs1: u32, rs2: u32) -> u32 { 0x33 | rd << 7 | rs1 << 15 | rs2 << 20 }
    fn lw(rd: u32, rs1: u32) -> u32 { 0x03 | rd << 7 | 2 << 12 | rs1 << 15 }
    fn sw(rs1: u32, rs2: u32) -> u32 { 0x23 | 2 << 12 | rs1 << 15 | rs2 << 20 }
    fn beq(rs1: u32, rs2: u32) -> u32 { 0x63 | rs1 << 15 | rs2 << 20 }
    fn jal(rd: u32) -> u32 { 0x6f | rd << 7 }
    fn jalr(rd: u32, rs1: u32) -> u32 { 0x67 | rd << 7 | rs1 << 15 }
    fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 { 0x33 | rd << 7 | rs1 << 15 | rs2 << 20 | 1 << 25 }
    fn div(rd: u32, rs1: u32, rs2: u32) -> u32 { 0x33 | rd << 7 | 4 << 12 | rs1 << 15 | rs2 << 20 | 1 << 25 }

    fn uop(inst: u32) -> Uop { Uop { info: classify(inst), istall: 0, dstall: 0, mispredict: false } }

    fn run(v: Vec<Uop>) -> u64 {
        let mut m = CycleModel::new();
        for u in v { m.push(u); }
        m.drain();
        m.cycles()
    }

    /// N independent instructions take N + 4 cycles (4 = pipeline fill).
    fn base(n: u64) -> u64 { n + 4 }

    #[test]
    fn independent_alu_ops_run_at_one_ipc() {
        let v: Vec<Uop> = (0..100).map(|i| uop(addi(1 + (i % 8), 20))).collect();
        assert_eq!(run(v), base(100));
    }

    #[test]
    fn alu_result_forwards_with_no_stall() {
        let v = vec![uop(addi(1, 0)), uop(add(2, 1, 1)), uop(add(3, 2, 2)), uop(add(4, 3, 3))];
        assert_eq!(run(v), base(4));
    }

    #[test]
    fn load_use_costs_exactly_one_cycle() {
        let v = vec![uop(lw(1, 2)), uop(add(3, 1, 1)), uop(addi(4, 0))];
        assert_eq!(run(v), base(3) + 1);
    }

    #[test]
    fn load_with_one_instruction_gap_has_no_stall() {
        let v = vec![uop(lw(1, 2)), uop(addi(5, 0)), uop(add(3, 1, 1))];
        assert_eq!(run(v), base(3));
    }

    #[test]
    fn load_feeding_store_data_does_not_stall() {
        let v = vec![uop(lw(1, 2)), uop(sw(3, 1)), uop(addi(4, 0))];
        assert_eq!(run(v), base(3));
    }

    #[test]
    fn load_feeding_store_address_stalls_one_cycle() {
        let v = vec![uop(lw(1, 2)), uop(sw(1, 3)), uop(addi(4, 0))];
        assert_eq!(run(v), base(3) + 1);
    }

    #[test]
    fn mispredicted_branch_costs_two_cycles() {
        let mut b = uop(beq(1, 2));
        b.mispredict = true;
        let v = vec![uop(addi(1, 0)), b, uop(addi(2, 0)), uop(addi(3, 0))];
        assert_eq!(run(v), base(4) + 2);
    }

    #[test]
    fn predicted_branch_is_free() {
        let v = vec![uop(addi(1, 0)), uop(beq(5, 6)), uop(addi(2, 0)), uop(addi(3, 0))];
        assert_eq!(run(v), base(4));
    }

    #[test]
    fn jal_costs_one_cycle_and_jalr_two() {
        assert_eq!(run(vec![uop(addi(1, 0)), uop(jal(0)), uop(addi(2, 0))]), base(3) + 1);
        assert_eq!(run(vec![uop(addi(1, 0)), uop(jalr(0, 5)), uop(addi(2, 0))]), base(3) + 2);
    }

    #[test]
    fn multiply_and_divide_latencies() {
        assert_eq!(run(vec![uop(mul(1, 2, 3)), uop(addi(2, 0))]), base(2) + 2);
        assert_eq!(run(vec![uop(div(1, 2, 3)), uop(addi(2, 0))]), base(2) + 19);
    }

    #[test]
    fn icache_miss_delays_by_its_latency() {
        let mut a = uop(addi(1, 0));
        a.istall = 10;
        assert_eq!(run(vec![uop(addi(2, 0)), a, uop(addi(3, 0))]), base(3) + 10);
    }

    #[test]
    fn dcache_miss_stalls_the_whole_pipeline() {
        let mut l = uop(lw(1, 2));
        l.dstall = 12;
        assert_eq!(run(vec![l, uop(addi(2, 0)), uop(addi(3, 0))]), base(3) + 12);
    }

    #[test]
    fn instruction_miss_overlaps_with_a_data_miss() {
        // While the load waits in MEM, the next instruction's I-miss proceeds in IF in parallel,
        // so the total is less than the sum of the two penalties (what a formula would charge).
        let mut l = uop(lw(1, 2));
        l.dstall = 10;
        let mut n = uop(addi(2, 0));
        n.istall = 10;
        let c = run(vec![l, n, uop(addi(3, 0))]);
        assert!(c < base(3) + 20, "cycles = {}", c);
        assert!(c >= base(3) + 10);
    }
}
