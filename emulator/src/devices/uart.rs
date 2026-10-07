//! 16550-style UART. TX goes to the host's stdout; RX comes either from a
//! preloaded input file (deterministic) or from a host stdin reader thread.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};

pub struct Uart {
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    dll: u8,
    dlm: u8,
    rx: VecDeque<u8>,
    chan: Option<Receiver<u8>>,
    thre_pending: bool,
    pub has_live_input: bool,
    pub tx_bytes: u64,
}

impl Uart {
    fn blank() -> Self {
        Uart {
            ier: 0, lcr: 0, mcr: 0, scr: 0, dll: 0, dlm: 0,
            rx: VecDeque::new(), chan: None, thre_pending: false,
            has_live_input: false, tx_bytes: 0,
        }
    }

    pub fn with_input(data: Vec<u8>) -> Self {
        let mut u = Self::blank();
        u.rx = data.into_iter().collect();
        u
    }

    pub fn with_stdin() -> Self {
        let mut u = Self::blank();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut b = [0u8; 1];
            while let Ok(1) = lock.read(&mut b) {
                if tx.send(b[0]).is_err() {
                    break;
                }
            }
        });
        u.chan = Some(rx);
        u.has_live_input = true;
        u
    }

    pub fn poll(&mut self) {
        if let Some(ch) = &self.chan {
            while let Ok(b) = ch.try_recv() {
                self.rx.push_back(b);
            }
        }
    }

    /// True if buffered input is waiting (used by the idle logic).
    pub fn rx_ready(&self) -> bool {
        !self.rx.is_empty()
    }

    pub fn irq(&self) -> bool {
        (self.ier & 1 != 0 && !self.rx.is_empty()) || (self.ier & 2 != 0 && self.thre_pending)
    }

    pub fn load(&mut self, off: u64, _size: usize) -> Option<u64> {
        let dlab = self.lcr & 0x80 != 0;
        Some(match off {
            0 if dlab => self.dll as u64,
            0 => self.rx.pop_front().unwrap_or(0) as u64,
            1 if dlab => self.dlm as u64,
            1 => self.ier as u64,
            2 => {
                if self.ier & 1 != 0 && !self.rx.is_empty() {
                    0xc4
                } else if self.ier & 2 != 0 && self.thre_pending {
                    self.thre_pending = false;
                    0xc2
                } else {
                    0xc1
                }
            }
            3 => self.lcr as u64,
            4 => self.mcr as u64,
            5 => 0x60 | (!self.rx.is_empty() as u64),
            6 => 0xb0,
            7 => self.scr as u64,
            _ => 0,
        })
    }

    pub fn store(&mut self, off: u64, _size: usize, val: u64) -> bool {
        let v = val as u8;
        let dlab = self.lcr & 0x80 != 0;
        match off {
            0 if dlab => self.dll = v,
            0 => {
                let out = std::io::stdout();
                let mut o = out.lock();
                let _ = o.write_all(&[v]);
                let _ = o.flush();
                self.tx_bytes += 1;
                self.thre_pending = true;
            }
            1 if dlab => self.dlm = v,
            1 => {
                if v & 2 != 0 && self.ier & 2 == 0 {
                    self.thre_pending = true;
                }
                self.ier = v & 0xf;
            }
            3 => self.lcr = v,
            4 => self.mcr = v,
            7 => self.scr = v,
            _ => {}
        }
        true
    }
}
