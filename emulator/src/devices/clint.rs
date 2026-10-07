//! CLINT: machine timer (mtime/mtimecmp) and software interrupt (msip). Single hart.

pub struct Clint {
    pub msip: u32,
    pub mtimecmp: u64,
    pub mtime: u64,
}

impl Clint {
    pub fn new() -> Self {
        Clint { msip: 0, mtimecmp: u64::MAX, mtime: 0 }
    }

    pub fn timer_pending(&self) -> bool {
        self.mtime >= self.mtimecmp
    }

    pub fn load(&self, off: u64, size: usize) -> Option<u64> {
        let (reg, base) = match off {
            0x0000..=0x0003 => (self.msip as u64, 0x0000),
            0x4000..=0x4007 => (self.mtimecmp, 0x4000),
            0xbff8..=0xbfff => (self.mtime, 0xbff8),
            _ => return Some(0),
        };
        let shift = (off - base) * 8;
        Some(mask(reg >> shift, size))
    }

    pub fn store(&mut self, off: u64, size: usize, val: u64) -> bool {
        let m = if size >= 8 { u64::MAX } else { (1u64 << (size * 8)) - 1 };
        match off {
            0x0000..=0x0003 => self.msip = (val & 1) as u32,
            0x4000..=0x4007 => {
                let sh = (off - 0x4000) * 8;
                self.mtimecmp = (self.mtimecmp & !(m << sh)) | ((val & m) << sh);
            }
            0xbff8..=0xbfff => {
                let sh = (off - 0xbff8) * 8;
                self.mtime = (self.mtime & !(m << sh)) | ((val & m) << sh);
            }
            _ => {}
        }
        true
    }
}

fn mask(v: u64, size: usize) -> u64 {
    if size >= 8 { v } else { v & ((1u64 << (size * 8)) - 1) }
}
