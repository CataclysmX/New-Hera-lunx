//! CRC16 used by SM messages (DLL `FUN_180016490` table + `FUN_180016570` loop).
//!
//! Reflected, table driven, init 0: `crc = (crc >> 8) ^ T[(crc ^ b) & 0xff]`.
//! The table is generated with a non-standard variant of polynomial 0x8005.

const fn make_table() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let b = i as u16;
        let neg = 0u16.wrapping_sub(b & 1); // 0x0000 or 0xffff
        let mut v: u16 = if (((neg & 5) ^ (b >> 1)) & 1) == 0 {
            neg & 0x4002
        } else {
            (neg & 0x4002) ^ 0x8005
        };
        let mut k = 2;
        while k < 8 {
            let mut nv = v >> 1;
            if (((b >> k) ^ v) & 1) != 0 {
                nv ^= 0x8005;
            }
            v = nv;
            k += 1;
        }
        t[i] = v;
        i += 1;
    }
    t
}

/// CRC lookup table.
pub const TABLE: [u16; 256] = make_table();

/// Continue a CRC16 computation over `data`.
pub fn crc16(data: &[u8], init: u16) -> u16 {
    data.iter().fold(init, |crc, &b| {
        (crc >> 8) ^ TABLE[((crc ^ b as u16) & 0xff) as usize]
    })
}
