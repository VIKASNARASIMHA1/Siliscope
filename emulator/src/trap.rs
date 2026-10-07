//! Exception causes and the Trap type used throughout the emulator.

pub const INSN_ACCESS: u64 = 1;
pub const ILLEGAL: u64 = 2;
pub const BREAKPOINT: u64 = 3;
pub const LOAD_MISALIGNED: u64 = 4;
pub const LOAD_ACCESS: u64 = 5;
pub const STORE_MISALIGNED: u64 = 6;
pub const STORE_ACCESS: u64 = 7;
pub const ECALL_U: u64 = 8;
pub const INSN_PAGE: u64 = 12;
pub const LOAD_PAGE: u64 = 13;
pub const STORE_PAGE: u64 = 15;

#[derive(Clone, Copy, Debug)]
pub struct Trap {
    pub cause: u64,
    pub tval: u64,
}

pub type Res<T> = Result<T, Trap>;

#[inline]
pub fn trap(cause: u64, tval: u64) -> Trap {
    Trap { cause, tval }
}
