//! Conditional-branch direction predictors (perfect BTB assumed).

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    NotTaken,
    Btfn,
    Bimodal,
    Gshare,
}

pub struct Predictor {
    kind: Kind,
    bits: usize,
    table: Vec<u8>, // 2-bit saturating counters
    hist: u64,
    pub total: u64,
    pub correct: u64,
}

impl Predictor {
    fn mk(kind: Kind, bits: usize) -> Self {
        let n = if matches!(kind, Kind::Bimodal | Kind::Gshare) { 1usize << bits } else { 0 };
        Predictor { kind, bits, table: vec![1; n], hist: 0, total: 0, correct: 0 }
    }
    pub fn static_not_taken() -> Self { Self::mk(Kind::NotTaken, 0) }
    pub fn static_btfn() -> Self { Self::mk(Kind::Btfn, 0) }
    pub fn bimodal(bits: usize) -> Self { Self::mk(Kind::Bimodal, bits) }
    pub fn gshare(bits: usize) -> Self { Self::mk(Kind::Gshare, bits) }

    pub fn name(&self) -> String {
        match self.kind {
            Kind::NotTaken => "static-not-taken".into(),
            Kind::Btfn => "static-btfn".into(),
            Kind::Bimodal => format!("bimodal-{}", 1usize << self.bits),
            Kind::Gshare => format!("gshare-{}b", self.bits),
        }
    }

    pub fn accuracy(&self) -> f64 {
        if self.total == 0 { 1.0 } else { self.correct as f64 / self.total as f64 }
    }

    /// Predicts, then trains. Returns true if the prediction was correct.
    pub fn predict_update(&mut self, pc: u64, taken: bool, backward: bool) -> bool {
        let pred = match self.kind {
            Kind::NotTaken => false,
            Kind::Btfn => backward,
            Kind::Bimodal | Kind::Gshare => {
                let mask = (1u64 << self.bits) - 1;
                let idx = if self.kind == Kind::Bimodal { (pc >> 1) & mask } else { ((pc >> 1) ^ self.hist) & mask } as usize;
                let c = self.table[idx];
                self.table[idx] = if taken { (c + 1).min(3) } else { c.saturating_sub(1) };
                if self.kind == Kind::Gshare {
                    self.hist = ((self.hist << 1) | taken as u64) & mask;
                }
                c >= 2
            }
        };
        self.total += 1;
        let ok = pred == taken;
        self.correct += ok as u64;
        ok
    }
}
