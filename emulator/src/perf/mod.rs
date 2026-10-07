//! Microarchitecture analyzer. It consumes the retired-instruction stream
//! (online, no trace file needed) and models:
//!   * branch predictors (static, bimodal, gshare),
//!   * a classic 5-stage in-order pipeline (load-use, mul/div, jump and
//!     mispredict penalties),
//!   * a 2-level cache hierarchy (L1I, L1D, unified L2; LRU, write-back),
//!   * optional design-space sweeps (many cache / predictor configs at once).

pub mod cache;
pub mod cycle;
pub mod pipeline;
pub mod predictor;

use cache::{Cache, Hierarchy};
use pipeline::{Mix, Pipe};
use predictor::Predictor;

/// One retired instruction, as seen by the analyzer.
#[derive(Clone, Copy, Default)]
pub struct Ev {
    pub pc: u64,
    pub inst: u32,
    pub len: u8,
    pub fetch_pa: u64,
    pub mem_pa: u64,
    pub mem_size: u8,
    /// 0 = none, 1 = load, 2 = store/AMO
    pub mem_kind: u8,
    pub next_pc: u64,
}

#[derive(Clone)]
pub struct PerfConfig {
    pub l1_kb: usize,
    pub l1_ways: usize,
    pub line: usize,
    pub l2_kb: usize,
    pub l2_ways: usize,
    pub l2_lat: u64,
    pub mem_lat: u64,
    pub sweep: bool,
    pub cycle: bool,
}

impl Default for PerfConfig {
    fn default() -> Self {
        PerfConfig { l1_kb: 16, l1_ways: 4, line: 64, l2_kb: 256, l2_ways: 8, l2_lat: 12, mem_lat: 100, sweep: false, cycle: false }
    }
}

struct SweepCache {
    kb: usize,
    ways: usize,
    line: usize,
    c: Cache,
}

struct SweepPred {
    name: String,
    param: usize,
    p: Predictor,
}

pub struct Perf {
    pub cfg: PerfConfig,
    pub mix: Mix,
    pipe: Pipe,
    hier: Hierarchy,
    preds: Vec<Predictor>,
    sweep_d: Vec<SweepCache>,
    sweep_i: Vec<SweepCache>,
    sweep_p: Vec<SweepPred>,
    cyc: Vec<cycle::CycleModel>,
}

impl Perf {
    pub fn new(cfg: PerfConfig) -> Self {
        let hier = Hierarchy::new(&cfg);
        let preds = vec![
            Predictor::static_not_taken(),
            Predictor::static_btfn(),
            Predictor::bimodal(10),
            Predictor::bimodal(12),
            Predictor::gshare(10),
            Predictor::gshare(14),
        ];
        let (mut sd, mut si, mut sp) = (Vec::new(), Vec::new(), Vec::new());
        if cfg.sweep {
            let push = |v: &mut Vec<SweepCache>, kb: usize, ways: usize, line: usize| {
                v.push(SweepCache { kb, ways, line, c: Cache::new(kb * 1024, ways, line) });
            };
            for &kb in &[1usize, 2, 4, 8, 16, 32, 64] {
                for &ways in &[1usize, 2, 4, 8] {
                    push(&mut sd, kb, ways, 64);
                    push(&mut si, kb, ways, 64);
                }
            }
            for &line in &[16usize, 32, 128, 256] {
                push(&mut sd, 16, 4, line);
                push(&mut si, 16, 4, line);
            }
            for bits in 4..=14usize {
                sp.push(SweepPred { name: "bimodal".into(), param: bits, p: Predictor::bimodal(bits) });
                sp.push(SweepPred { name: "gshare".into(), param: bits, p: Predictor::gshare(bits) });
            }
        }
        let cyc = if cfg.cycle { (0..preds.len()).map(|_| cycle::CycleModel::new()).collect() } else { Vec::new() };
        Perf { cfg, mix: Mix::default(), pipe: Pipe::new(preds.len()), hier, preds, sweep_d: sd, sweep_i: si, sweep_p: sp, cyc }
    }

    pub fn record(&mut self, e: &Ev) {
        let info = pipeline::classify(e.inst);
        self.mix.count(&info);
        // Front end: instruction cache
        let istall = self.hier.fetch(e.fetch_pa);
        // Memory stage
        let mut dstall = 0;
        if e.mem_kind != 0 {
            dstall = self.hier.data(e.mem_pa, e.mem_kind == 2);
        }
        // Branch prediction
        let mut mis = [false; 8];
        if info.cond_branch {
            let taken = e.next_pc != e.pc + e.len as u64;
            let backward = (e.inst >> 31) & 1 == 1;
            self.mix.taken += taken as u64;
            for (i, p) in self.preds.iter_mut().enumerate() {
                mis[i] = !p.predict_update(e.pc, taken, backward);
            }
            for s in self.sweep_p.iter_mut() {
                s.p.predict_update(e.pc, taken, backward);
            }
        }
        self.pipe.step(&info, istall, dstall, &mis[..self.preds.len()]);
        for (i, cm) in self.cyc.iter_mut().enumerate() {
            cm.push(cycle::Uop { info, istall: istall as u32, dstall: dstall as u32, mispredict: mis[i] });
        }
        if self.cfg.sweep {
            for s in self.sweep_i.iter_mut() {
                s.c.access(e.fetch_pa, false);
            }
            if e.mem_kind != 0 {
                for s in self.sweep_d.iter_mut() {
                    s.c.access(e.mem_pa, e.mem_kind == 2);
                }
            }
        }
    }

    /// Drain the cycle-accurate pipelines (call once, after the last instruction).
    pub fn finish(&mut self) {
        for cm in self.cyc.iter_mut() {
            cm.drain();
        }
    }

    /// Human-readable report.
    pub fn report(&self, tlb: Option<(u64, u64)>) -> String {
        let mut s = String::new();
        let n = self.mix.instrs.max(1);
        let pct = |x: u64| 100.0 * x as f64 / n as f64;
        s += "\n==================== rvsim performance report ====================\n";
        s += &format!("Instructions retired : {}\n", self.mix.instrs);
        s += &format!(
            "Mix                  : load {:.1}%  store {:.1}%  cond-branch {:.1}%  jal/jalr {:.1}%  mul {:.1}%  div {:.1}%  amo {:.1}%  csr/sys {:.1}%\n",
            pct(self.mix.loads), pct(self.mix.stores), pct(self.mix.branches), pct(self.mix.jumps),
            pct(self.mix.muls), pct(self.mix.divs), pct(self.mix.amos), pct(self.mix.system)
        );
        s += &format!("\n--- 5-stage pipeline model ---\n");
        s += &format!("Base cycles (1/instr)   : {}\n", self.pipe.base);
        s += &format!("Load-use stalls         : {}\n", self.pipe.load_use);
        s += &format!("Mul/div latency         : {}\n", self.pipe.muldiv);
        s += &format!("Jump bubbles            : {}\n", self.pipe.jump);
        s += &format!("I-cache stall cycles    : {}\n", self.pipe.istall);
        s += &format!("D-cache stall cycles    : {}\n", self.pipe.dstall);
        s += &format!("\n--- Branch prediction ({} conditional branches, {:.1}% taken) ---\n", self.mix.branches,
            100.0 * self.mix.taken as f64 / self.mix.branches.max(1) as f64);
        s += &format!("{:<18} {:>9} {:>12} {:>8}\n", "predictor", "accuracy", "mispredicts", "CPI");
        for (i, p) in self.preds.iter().enumerate() {
            s += &format!("{:<18} {:>8.2}% {:>12} {:>8.3}\n", p.name(), p.accuracy() * 100.0, p.total - p.correct, self.pipe.cpi(i, n));
        }
        if !self.cyc.is_empty() {
            s += "\n--- Validation: analytic (formula) CPI vs cycle-by-cycle pipeline simulation ---\n";
            s += &format!("{:<18} {:>10} {:>12} {:>9}\n", "predictor", "formula", "cycle-accurate", "diff");
            for (i, p) in self.preds.iter().enumerate() {
                let a = self.pipe.cpi(i, n);
                let c = self.cyc[i].cpi();
                s += &format!("{:<18} {:>10.4} {:>12.4} {:>+8.2}%\n", p.name(), a, c, 100.0 * (a - c) / c);
            }
            let st = self.cyc[self.cyc.len() - 1].stalls;
            s += &format!("cycle-sim stall cycles (last predictor): operand waits {}, EX busy {}, fetch idle (redirect) {}, I-miss {}, D-miss {}\n",
                st.load_use, st.ex_busy, st.fetch_blocked, st.imiss, st.dmiss);
        }
        s += "\n--- Cache hierarchy ---\n";
        s += &format!("L1I {:>4} KiB {}-way {}B : accesses {:>10}  miss rate {:>6.2}%  AMAT {:.2} cycles\n",
            self.cfg.l1_kb, self.cfg.l1_ways, self.cfg.line, self.hier.l1i.accesses, self.hier.l1i.miss_rate() * 100.0, self.hier.amat(true));
        s += &format!("L1D {:>4} KiB {}-way {}B : accesses {:>10}  miss rate {:>6.2}%  AMAT {:.2} cycles\n",
            self.cfg.l1_kb, self.cfg.l1_ways, self.cfg.line, self.hier.l1d.accesses, self.hier.l1d.miss_rate() * 100.0, self.hier.amat(false));
        s += &format!("L2  {:>4} KiB {}-way {}B : accesses {:>10}  miss rate {:>6.2}%  writebacks {}\n",
            self.cfg.l2_kb, self.cfg.l2_ways, self.cfg.line, self.hier.l2.accesses, self.hier.l2.miss_rate() * 100.0, self.hier.l2.writebacks);
        s += &format!("(latencies: L1 hit 1, L2 hit {}, memory {} cycles)\n", self.cfg.l2_lat, self.cfg.mem_lat);
        if let Some((h, m)) = tlb {
            if h + m > 0 {
                s += &format!("\n--- TLB (256-entry direct-mapped) ---\nlookups {}  hit rate {:.3}%\n", h + m, 100.0 * h as f64 / (h + m) as f64);
            }
        }
        s += "====================================================================\n";
        s
    }

    /// Write machine-readable CSV files for plotting: <prefix>_summary.csv and,
    /// when sweeping, <prefix>_dcache.csv, <prefix>_icache.csv, <prefix>_pred.csv
    pub fn write_csv(&self, prefix: &str, tlb: Option<(u64, u64)>) -> std::io::Result<()> {
        let n = self.mix.instrs.max(1);
        let mut s = String::from("metric,value\n");
        let mut kv = |k: &str, v: String| s += &format!("{},{}\n", k, v);
        kv("instructions", self.mix.instrs.to_string());
        kv("loads", self.mix.loads.to_string());
        kv("stores", self.mix.stores.to_string());
        kv("cond_branches", self.mix.branches.to_string());
        kv("taken_branches", self.mix.taken.to_string());
        kv("jumps", self.mix.jumps.to_string());
        kv("muls", self.mix.muls.to_string());
        kv("divs", self.mix.divs.to_string());
        kv("l1i_miss_rate", format!("{:.6}", self.hier.l1i.miss_rate()));
        kv("l1d_miss_rate", format!("{:.6}", self.hier.l1d.miss_rate()));
        kv("l2_miss_rate", format!("{:.6}", self.hier.l2.miss_rate()));
        kv("amat_i", format!("{:.4}", self.hier.amat(true)));
        kv("amat_d", format!("{:.4}", self.hier.amat(false)));
        if let Some((h, m)) = tlb {
            kv("tlb_hit_rate", format!("{:.6}", if h + m > 0 { h as f64 / (h + m) as f64 } else { 1.0 }));
        }
        for (i, p) in self.preds.iter().enumerate() {
            kv(&format!("accuracy_{}", p.name()), format!("{:.6}", p.accuracy()));
            kv(&format!("cpi_{}", p.name()), format!("{:.4}", self.pipe.cpi(i, n)));
            if !self.cyc.is_empty() {
                kv(&format!("cyclecpi_{}", p.name()), format!("{:.4}", self.cyc[i].cpi()));
            }
        }
        std::fs::write(format!("{}_summary.csv", prefix), s)?;
        if self.cfg.sweep {
            for (name, v) in [("dcache", &self.sweep_d), ("icache", &self.sweep_i)] {
                let mut t = String::from("size_kb,ways,line,accesses,miss_rate\n");
                for c in v.iter() {
                    t += &format!("{},{},{},{},{:.6}\n", c.kb, c.ways, c.line, c.c.accesses, c.c.miss_rate());
                }
                std::fs::write(format!("{}_{}.csv", prefix, name), t)?;
            }
            let mut t = String::from("predictor,bits,table_entries,accuracy\n");
            for p in &self.sweep_p {
                t += &format!("{},{},{},{:.6}\n", p.name, p.param, 1usize << p.param, p.p.accuracy());
            }
            std::fs::write(format!("{}_pred.csv", prefix), t)?;
        }
        Ok(())
    }
}
