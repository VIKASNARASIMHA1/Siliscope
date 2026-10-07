//! Expansion of 16-bit compressed (RVC) instructions into their 32-bit
//! equivalents (RV64C, no floating-point forms).

fn enc_r(op: u32, rd: u32, f3: u32, rs1: u32, rs2: u32, f7: u32) -> u32 {
    op | rd << 7 | f3 << 12 | rs1 << 15 | rs2 << 20 | f7 << 25
}
fn enc_i(op: u32, rd: u32, f3: u32, rs1: u32, imm: i32) -> u32 {
    op | rd << 7 | f3 << 12 | rs1 << 15 | ((imm as u32) & 0xfff) << 20
}
fn enc_s(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let i = imm as u32;
    op | (i & 0x1f) << 7 | f3 << 12 | rs1 << 15 | rs2 << 20 | ((i >> 5) & 0x7f) << 25
}
fn enc_b(f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let i = imm as u32;
    0x63 | ((i >> 11) & 1) << 7 | ((i >> 1) & 0xf) << 8 | f3 << 12 | rs1 << 15 | rs2 << 20
        | ((i >> 5) & 0x3f) << 25 | ((i >> 12) & 1) << 31
}
fn enc_j(rd: u32, imm: i32) -> u32 {
    let i = imm as u32;
    0x6f | rd << 7 | ((i >> 12) & 0xff) << 12 | ((i >> 11) & 1) << 20 | ((i >> 1) & 0x3ff) << 21
        | ((i >> 20) & 1) << 31
}
fn enc_u(rd: u32, imm: i32) -> u32 {
    0x37 | rd << 7 | ((imm as u32) & 0xfffff000)
}
fn sext(v: u32, bits: u32) -> i32 {
    ((v << (32 - bits)) as i32) >> (32 - bits)
}
fn bit(i: u32, n: u32) -> u32 {
    (i >> n) & 1
}
fn bits(i: u32, hi: u32, lo: u32) -> u32 {
    (i >> lo) & ((1 << (hi - lo + 1)) - 1)
}

pub fn expand(c: u16) -> Option<u32> {
    let i = c as u32;
    let f3 = bits(i, 15, 13);
    let rd_full = bits(i, 11, 7);
    let rs2_full = bits(i, 6, 2);
    let rdp = 8 + bits(i, 4, 2); // rd' / rs2'
    let rs1p = 8 + bits(i, 9, 7); // rs1' / rd'
    match i & 3 {
        0 => match f3 {
            0 => {
                // C.ADDI4SPN
                let nz = bits(i, 12, 11) << 4 | bits(i, 10, 7) << 6 | bit(i, 6) << 2 | bit(i, 5) << 3;
                if nz == 0 {
                    return None;
                }
                Some(enc_i(0x13, rdp, 0, 2, nz as i32))
            }
            2 => {
                let off = bits(i, 12, 10) << 3 | bit(i, 6) << 2 | bit(i, 5) << 6;
                Some(enc_i(0x03, rdp, 2, rs1p, off as i32))
            }
            3 => {
                let off = bits(i, 12, 10) << 3 | bits(i, 6, 5) << 6;
                Some(enc_i(0x03, rdp, 3, rs1p, off as i32))
            }
            6 => {
                let off = bits(i, 12, 10) << 3 | bit(i, 6) << 2 | bit(i, 5) << 6;
                Some(enc_s(0x23, 2, rs1p, rdp, off as i32))
            }
            7 => {
                let off = bits(i, 12, 10) << 3 | bits(i, 6, 5) << 6;
                Some(enc_s(0x23, 3, rs1p, rdp, off as i32))
            }
            _ => None,
        },
        1 => {
            let imm6 = sext(bit(i, 12) << 5 | rs2_full, 6);
            match f3 {
                0 => Some(enc_i(0x13, rd_full, 0, rd_full, imm6)),
                1 => {
                    if rd_full == 0 {
                        return None;
                    }
                    Some(enc_i(0x1b, rd_full, 0, rd_full, imm6))
                }
                2 => Some(enc_i(0x13, rd_full, 0, 0, imm6)),
                3 => {
                    if rd_full == 2 {
                        let imm = sext(
                            bit(i, 12) << 9 | bit(i, 6) << 4 | bit(i, 5) << 6 | bits(i, 4, 3) << 7 | bit(i, 2) << 5,
                            10,
                        );
                        if imm == 0 {
                            return None;
                        }
                        Some(enc_i(0x13, 2, 0, 2, imm))
                    } else {
                        let imm = sext(bit(i, 12) << 17 | rs2_full << 12, 18);
                        if imm == 0 || rd_full == 0 {
                            return None;
                        }
                        Some(enc_u(rd_full, imm))
                    }
                }
                4 => {
                    let shamt = bit(i, 12) << 5 | rs2_full;
                    match bits(i, 11, 10) {
                        0 => Some(enc_i(0x13, rs1p, 5, rs1p, shamt as i32)),
                        1 => Some(enc_i(0x13, rs1p, 5, rs1p, (shamt | 0x400) as i32)),
                        2 => Some(enc_i(0x13, rs1p, 7, rs1p, imm6)),
                        _ => {
                            let sel = bits(i, 6, 5);
                            if bit(i, 12) == 0 {
                                match sel {
                                    0 => Some(enc_r(0x33, rs1p, 0, rs1p, rdp, 0x20)),
                                    1 => Some(enc_r(0x33, rs1p, 4, rs1p, rdp, 0)),
                                    2 => Some(enc_r(0x33, rs1p, 6, rs1p, rdp, 0)),
                                    _ => Some(enc_r(0x33, rs1p, 7, rs1p, rdp, 0)),
                                }
                            } else {
                                match sel {
                                    0 => Some(enc_r(0x3b, rs1p, 0, rs1p, rdp, 0x20)),
                                    1 => Some(enc_r(0x3b, rs1p, 0, rs1p, rdp, 0)),
                                    _ => None,
                                }
                            }
                        }
                    }
                }
                5 => {
                    let off = sext(
                        bit(i, 12) << 11 | bit(i, 11) << 4 | bits(i, 10, 9) << 8 | bit(i, 8) << 10 | bit(i, 7) << 6
                            | bit(i, 6) << 7 | bits(i, 5, 3) << 1 | bit(i, 2) << 5,
                        12,
                    );
                    Some(enc_j(0, off))
                }
                _ => {
                    let off = sext(
                        bit(i, 12) << 8 | bits(i, 11, 10) << 3 | bits(i, 6, 5) << 6 | bits(i, 4, 3) << 1 | bit(i, 2) << 5,
                        9,
                    );
                    Some(enc_b(if f3 == 6 { 0 } else { 1 }, rs1p, 0, off))
                }
            }
        }
        2 => match f3 {
            0 => {
                if rd_full == 0 {
                    return None;
                }
                Some(enc_i(0x13, rd_full, 1, rd_full, (bit(i, 12) << 5 | rs2_full) as i32))
            }
            2 => {
                if rd_full == 0 {
                    return None;
                }
                let off = bit(i, 12) << 5 | bits(i, 6, 4) << 2 | bits(i, 3, 2) << 6;
                Some(enc_i(0x03, rd_full, 2, 2, off as i32))
            }
            3 => {
                if rd_full == 0 {
                    return None;
                }
                let off = bit(i, 12) << 5 | bits(i, 6, 5) << 3 | bits(i, 4, 2) << 6;
                Some(enc_i(0x03, rd_full, 3, 2, off as i32))
            }
            4 => {
                if bit(i, 12) == 0 {
                    if rs2_full == 0 {
                        if rd_full == 0 {
                            return None;
                        }
                        Some(enc_i(0x67, 0, 0, rd_full, 0)) // C.JR
                    } else {
                        Some(enc_r(0x33, rd_full, 0, 0, rs2_full, 0)) // C.MV
                    }
                } else if rs2_full == 0 {
                    if rd_full == 0 {
                        Some(0x0010_0073) // C.EBREAK
                    } else {
                        Some(enc_i(0x67, 1, 0, rd_full, 0)) // C.JALR
                    }
                } else {
                    Some(enc_r(0x33, rd_full, 0, rd_full, rs2_full, 0)) // C.ADD
                }
            }
            6 => {
                let off = bits(i, 12, 9) << 2 | bits(i, 8, 7) << 6;
                Some(enc_s(0x23, 2, 2, rs2_full, off as i32))
            }
            7 => {
                let off = bits(i, 12, 10) << 3 | bits(i, 9, 7) << 6;
                Some(enc_s(0x23, 3, 2, rs2_full, off as i32))
            }
            _ => None,
        },
        _ => None,
    }
}
