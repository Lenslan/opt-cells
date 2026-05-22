/// Truth table for a function of up to 6 inputs, stored in a u64.
///
/// Bit i of the u64 corresponds to input pattern i (i.e., for k inputs,
/// bit i represents the output when the inputs equal the binary representation of i;
/// bit 0 of i is the value of input 0, bit 1 is the value of input 1, etc.).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tt64(pub u64);

impl Tt64 {
    pub const ZERO: Tt64 = Tt64(0);
    pub const ONE_K0: Tt64 = Tt64(0x1); // constant 1 with k=0 (mask = 1 bit)

    /// Mask of valid bits for k inputs (2^(2^k) - 1 for k ≤ 5, all bits for k = 6).
    pub fn mask(k: u32) -> u64 {
        match k {
            0 => 0x1,
            1 => 0x3,
            2 => 0xF,
            3 => 0xFF,
            4 => 0xFFFF,
            5 => 0xFFFF_FFFF,
            6 => 0xFFFF_FFFF_FFFF_FFFF,
            _ => panic!("k > 6 not supported"),
        }
    }

    /// Truth table of input variable `i` (0-indexed) over k inputs.
    /// For k inputs, bit pattern p has bit i set iff input i = 1 in pattern p.
    pub fn var(i: u32, k: u32) -> Tt64 {
        let mut tt: u64 = 0;
        let n = 1u64 << k;
        for p in 0..n {
            if (p >> i) & 1 == 1 {
                tt |= 1u64 << p;
            }
        }
        Tt64(tt & Self::mask(k))
    }

    pub fn and(self, other: Tt64) -> Tt64 {
        Tt64(self.0 & other.0)
    }
    pub fn or(self, other: Tt64) -> Tt64 {
        Tt64(self.0 | other.0)
    }
    pub fn xor(self, other: Tt64) -> Tt64 {
        Tt64(self.0 ^ other.0)
    }
    pub fn not_in_k(self, k: u32) -> Tt64 {
        Tt64((!self.0) & Self::mask(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_widths() {
        assert_eq!(Tt64::mask(2), 0xF);
        assert_eq!(Tt64::mask(3), 0xFF);
        assert_eq!(Tt64::mask(4), 0xFFFF);
    }

    #[test]
    fn var_k2() {
        // 2 inputs: 4 patterns (00, 01, 10, 11)
        // var 0 = LSB of pattern: 0,1,0,1 → bits 1 and 3 set → 0b1010 = 0xA
        // var 1 = bit 1 of pattern: 0,0,1,1 → bits 2 and 3 set → 0b1100 = 0xC
        assert_eq!(Tt64::var(0, 2), Tt64(0xA));
        assert_eq!(Tt64::var(1, 2), Tt64(0xC));
    }

    #[test]
    fn and_or_xor() {
        let a = Tt64::var(0, 2);
        let b = Tt64::var(1, 2);
        assert_eq!(a.and(b), Tt64(0x8)); // a AND b = 1 only at pattern 11
        assert_eq!(a.or(b), Tt64(0xE)); // a OR b = 1 at 01, 10, 11
        assert_eq!(a.xor(b), Tt64(0x6)); // a XOR b = 1 at 01, 10
    }

    #[test]
    fn not_in_k() {
        let a = Tt64::var(0, 2);
        assert_eq!(a.not_in_k(2), Tt64(0x5)); // ~a in k=2 = 1 at 00, 10
    }
}
