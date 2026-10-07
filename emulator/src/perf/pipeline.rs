//! Analytical 5-stage in-order pipeline (IF ID EX MEM WB) with full
//! forwarding. Penalties:
//!   load-use hazard      : 1 bubble
//!   mul / div            : +2 / +19 extra cycles (3 / 20 total latency)
//!   jal / jalr           : 1 / 2 bubbles (target known in ID / EX)
//!   cond. mispredict     : 2 bubbles (resolved in EX)
//!   cache misses         : L2 / memory latency on the I and D side

pub const MISPREDICT_PENALTY: u64 = 2;
pub const MUL_EXTRA: u64 = 2;
pub const DIV_EXTRA: u64 = 19;

#[derive(Default, Clone, Copy)]
pub struct Info {
    pub load: bool,
    pub store: bool,
    pub cond_branch: bool,
    pub jal: bool,
    pub jalr: bool,
    pub mul: bool,
    pub div: bool,
    pub amo: bool,
    pub system: bool,
    pub rd: u8,
    pub rs1: u8,
    pub rs2: u8,
    pub uses_rs1: bool,
    pub uses_rs2: bool,
    pub writes_rd: bool,
}

pub fn classify(inst: u32) -> Info {
    let op = inst & 0x7f;
    let f3 = (inst >> 12) & 7;
    let f7 = inst >> 25;
    let mut i = Info { rd: ((inst >> 7) & 31) as u8, rs1: ((inst >> 15) & 31) as u8, rs2: ((inst >> 20) & 31) as u8, ..Default::default() };
    match op {
        0x03 => { i.load = true; i.uses_rs1 = true; i.writes_rd = true; }
        0x23 => { i.store = true; i.uses_rs1 = true; i.uses_rs2 = true; }
        0x63 => { i.cond_branch = true; i.uses_rs1 = true; i.uses_rs2 = true; }
        0x6f => { i.jal = true; i.writes_rd = true; }
        0x67 => { i.jalr = true; i.uses_rs1 = true; i.writes_rd = true; }
        0x13 | 0x1b => { i.uses_rs1 = true; i.writes_rd = true; }
        0x33 | 0x3b => {
            i.uses_rs1 = true; i.uses_rs2 = true; i.writes_rd = true;
            if f7 == 1 { if f3 >= 4 { i.div = true } else { i.mul = true } }
        }
        0x2f => { i.amo = true; i.load = true; i.store = true; i.uses_rs1 = true; i.uses_rs2 = true; i.writes_rd = true; }
        0x37 | 0x17 => { i.writes_rd = true; }
        0x73 => { i.system = true; i.uses_rs1 = f3 != 0 && f3 < 4; i.writes_rd = f3 != 0; }
        _ => {}
    }
    i
}

#[derive(Default)]
pub struct Mix {
    pub instrs: u64,
    pub loads: u64,
    pub stores: u64,
    pub branches: u64,
    pub taken: u64,
    pub jumps: u64,
    pub muls: u64,
    pub divs: u64,
    pub amos: u64,
    pub system: u64,
}

impl Mix {
    pub fn count(&mut self, i: &Info) {
        self.instrs += 1;
        if i.amo { self.amos += 1 } else if i.load { self.loads += 1 } else if i.store { self.stores += 1 }
        self.branches += i.cond_branch as u64;
        self.jumps += (i.jal || i.jalr) as u64;
        self.muls += i.mul as u64;
        self.divs += i.div as u64;
        self.system += i.system as u64;
    }
}

pub struct Pipe {
    pub base: u64,
    pub load_use: u64,
    pub muldiv: u64,
    pub jump: u64,
    pub istall: u64,
    pub dstall: u64,
    mispredicts: Vec<u64>,
    prev_load_rd: u8,
}

impl Pipe {
    pub fn new(npred: usize) -> Self {
        Pipe { base: 0, load_use: 0, muldiv: 0, jump: 0, istall: 0, dstall: 0, mispredicts: vec![0; npred], prev_load_rd: 0 }
    }

    pub fn step(&mut self, i: &Info, istall: u64, dstall: u64, mis: &[bool]) {
        self.base += 1;
        let p = self.prev_load_rd;
        if p != 0 && ((i.uses_rs1 && i.rs1 == p) || (i.uses_rs2 && i.rs2 == p)) {
            self.load_use += 1;
        }
        self.prev_load_rd = if i.load && !i.store && i.rd != 0 { i.rd } else { 0 };
        if i.mul { self.muldiv += MUL_EXTRA }
        if i.div { self.muldiv += DIV_EXTRA }
        if i.jal { self.jump += 1 }
        if i.jalr { self.jump += 2 }
        self.istall += istall;
        self.dstall += dstall;
        for (k, m) in mis.iter().enumerate() {
            self.mispredicts[k] += *m as u64;
        }
    }

    pub fn cpi(&self, pred: usize, instrs: u64) -> f64 {
        let cyc = self.base + self.load_use + self.muldiv + self.jump + self.istall + self.dstall
            + self.mispredicts[pred] * MISPREDICT_PENALTY;
        cyc as f64 / instrs as f64
    }
}
