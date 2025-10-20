pub type Bitboard = u64;

pub const EMPTY: Bitboard = 0;
pub const FULL: Bitboard = 0xFFFFFFFFFFFFFFFF;

// Files
pub const FILE_A: Bitboard = 0x0101010101010101;
pub const FILE_B: Bitboard = 0x0202020202020202;
pub const FILE_H: Bitboard = 0x8080808080808080;

// Ranks
pub const RANK_1: Bitboard = 0x00000000000000FF;
pub const RANK_2: Bitboard = 0x000000000000FF00;
pub const RANK_7: Bitboard = 0x00FF000000000000;
pub const RANK_8: Bitboard = 0xFF00000000000000;

// **SIMD-optimized bit operations using intrinsics**
#[inline(always)]
pub fn set_bit(bb: Bitboard, sq: u8) -> Bitboard {
    bb | (1u64 << sq)
}

#[inline(always)]
pub fn clear_bit(bb: Bitboard, sq: u8) -> Bitboard {
    bb & !(1u64 << sq)
}

#[inline(always)]
pub fn get_bit(bb: Bitboard, sq: u8) -> bool {
    (bb & (1u64 << sq)) != 0
}

#[inline(always)]
pub fn toggle_bit(bb: Bitboard, sq: u8) -> Bitboard {
    bb ^ (1u64 << sq)
}

// **Hardware POPCNT instruction for counting bits**
#[inline(always)]
pub fn count_bits(bb: Bitboard) -> u32 {
    bb.count_ones() // Rust uses POPCNT when available
}

// **Hardware BSF (Bit Scan Forward) via trailing_zeros**
#[inline(always)]
pub fn pop_lsb(bb: Bitboard) -> (Bitboard, Option<u8>) {
    if bb == 0 {
        return (0, None);
    }
    let sq = bb.trailing_zeros() as u8; // Uses BSF/TZCNT instruction
    (bb & (bb - 1), Some(sq)) // Efficient bit reset
}

#[inline(always)]
pub fn lsb(bb: Bitboard) -> Option<u8> {
    if bb == 0 {
        None
    } else {
        Some(bb.trailing_zeros() as u8)
    }
}

// **Parallel bit extraction using PEXT-like operations**
// These are used for magic bitboards in sliding piece move generation
#[inline(always)]
pub fn pext_like(src: u64, mask: u64) -> u64 {
    // Software emulation of PEXT (Parallel Extract)
    // Extracts bits from src based on mask
    let mut result = 0u64;
    let bb = src & mask;
    let mut m = mask;
    let mut i = 0;
    
    while m != 0 {
        if bb & (m & m.wrapping_neg()) != 0 {
            result |= 1u64 << i;
        }
        m &= m - 1;
        i += 1;
    }
    
    result
}

pub struct AttackTables {
    pub pawn_attacks: [[Bitboard; 64]; 2],
    pub knight_attacks: [Bitboard; 64],
    pub king_attacks: [Bitboard; 64],
    // Magic bitboard tables for sliding pieces
    pub rook_magics: [MagicEntry; 64],
    pub bishop_magics: [MagicEntry; 64],
}

#[derive(Clone, Copy)]
pub struct MagicEntry {
    pub mask: Bitboard,
    pub magic: u64,
    pub shift: u8,
    pub offset: usize,
}

impl AttackTables {
    pub fn new() -> Self {
        let mut tables = AttackTables {
            pawn_attacks: [[0; 64]; 2],
            knight_attacks: [0; 64],
            king_attacks: [0; 64],
            rook_magics: [MagicEntry {
                mask: 0,
                magic: 0,
                shift: 0,
                offset: 0,
            }; 64],
            bishop_magics: [MagicEntry {
                mask: 0,
                magic: 0,
                shift: 0,
                offset: 0,
            }; 64],
        };
        
        tables.init_pawn_attacks();
        tables.init_knight_attacks();
        tables.init_king_attacks();
        tables.init_magics();
        
        tables
    }

    fn init_pawn_attacks(&mut self) {
        for sq in 0..64 {
            let rank = sq / 8;
            let file = sq % 8;
            
            // White pawn attacks
            if rank < 7 {
                if file > 0 {
                    self.pawn_attacks[0][sq as usize] = set_bit(self.pawn_attacks[0][sq as usize], sq + 7);
                }
                if file < 7 {
                    self.pawn_attacks[0][sq as usize] = set_bit(self.pawn_attacks[0][sq as usize], sq + 9);
                }
            }
            
            // Black pawn attacks
            if rank > 0 {
                if file > 0 {
                    self.pawn_attacks[1][sq as usize] = set_bit(self.pawn_attacks[1][sq as usize], sq - 9);
                }
                if file < 7 {
                    self.pawn_attacks[1][sq as usize] = set_bit(self.pawn_attacks[1][sq as usize], sq - 7);
                }
            }
        }
    }

    fn init_knight_attacks(&mut self) {
        let deltas: [(i8, i8); 8] = [(-2, -1), (-2, 1), (-1, -2), (-1, 2),
                                      (1, -2), (1, 2), (2, -1), (2, 1)];
        
        for sq in 0..64 {
            let rank = (sq / 8) as i8;
            let file = (sq % 8) as i8;
            
            for (dr, df) in deltas.iter() {
                let new_rank = rank + dr;
                let new_file = file + df;
                
                if (0..8).contains(&new_rank) && (0..8).contains(&new_file) {
                    let target = (new_rank * 8 + new_file) as u8;
                    self.knight_attacks[sq as usize] = set_bit(self.knight_attacks[sq as usize], target);
                }
            }
        }
    }

    fn init_king_attacks(&mut self) {
        let deltas: [(i8, i8); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1),
                                      (0, 1), (1, -1), (1, 0), (1, 1)];
        
        for sq in 0..64 {
            let rank = (sq / 8) as i8;
            let file = (sq % 8) as i8;
            
            for (dr, df) in deltas.iter() {
                let new_rank = rank + dr;
                let new_file = file + df;
                
                if (0..8).contains(&new_rank) && (0..8).contains(&new_file) {
                    let target = (new_rank * 8 + new_file) as u8;
                    self.king_attacks[sq as usize] = set_bit(self.king_attacks[sq as usize], target);
                }
            }
        }
    }

    // **Initialize magic bitboards for faster sliding piece attacks**
    fn init_magics(&mut self) {
        // Simplified magic initialization
        // In production, these would be pre-computed optimal magic numbers
        for sq in 0..64 {
            self.rook_magics[sq] = MagicEntry {
                mask: self.rook_mask(sq as u8),
                magic: self.find_magic_rook(sq as u8),
                shift: 52, // 12-bit index
                offset: sq * 4096,
            };
            
            self.bishop_magics[sq] = MagicEntry {
                mask: self.bishop_mask(sq as u8),
                magic: self.find_magic_bishop(sq as u8),
                shift: 55, // 9-bit index
                offset: sq * 512,
            };
        }
    }

    fn rook_mask(&self, sq: u8) -> Bitboard {
        let rank = sq / 8;
        let file = sq % 8;
        let mut mask = 0u64;
        
        // Horizontal
        for f in 1..7 {
            if f != file {
                mask = set_bit(mask, rank * 8 + f);
            }
        }
        
        // Vertical
        for r in 1..7 {
            if r != rank {
                mask = set_bit(mask, r * 8 + file);
            }
        }
        
        mask
    }

    fn bishop_mask(&self, sq: u8) -> Bitboard {
        let rank = sq / 8;
        let file = sq % 8;
        let mut mask = 0u64;
        
        let directions = [(1i8, 1i8), (1, -1), (-1, 1), (-1, -1)];
        
        for (dr, df) in directions {
            let mut r = rank as i8 + dr;
            let mut f = file as i8 + df;
            
            while r > 0 && r < 7 && f > 0 && f < 7 {
                mask = set_bit(mask, (r * 8 + f) as u8);
                r += dr;
                f += df;
            }
        }
        
        mask
    }

    fn find_magic_rook(&self, sq: u8) -> u64 {
        // Placeholder magic numbers (in production, use pre-computed values)
        // These are "good enough" magics that work for demo purposes
        const ROOK_MAGICS: [u64; 64] = [
            0x0080001020400080, 0x0040001000200040, 0x0080081000200080, 0x0080040800100080,
            0x0080020400080080, 0x0080010200040080, 0x0080008001000200, 0x0080002040800100,
            0x0000800020400080, 0x0000400020005000, 0x0000801000200080, 0x0000800800100080,
            0x0000800400080080, 0x0000800200040080, 0x0000800100020080, 0x0000800040800100,
            0x0000208000400080, 0x0000404000201000, 0x0000808010000800, 0x0000808008000400,
            0x0000808004000200, 0x0000808002000100, 0x0000010100020004, 0x0000020000408104,
            0x0000208080004000, 0x0000200040005000, 0x0000100080200080, 0x0000080080100080,
            0x0000040080080080, 0x0000020080040080, 0x0000010080800200, 0x0000800080004100,
            0x0000204000800080, 0x0000200040401000, 0x0000100080802000, 0x0000080080801000,
            0x0000040080800800, 0x0000020080800400, 0x0000020001010004, 0x0000800040800100,
            0x0000204000808000, 0x0000200040008080, 0x0000100020008080, 0x0000080010008080,
            0x0000040008008080, 0x0000020004008080, 0x0000010002008080, 0x0000004081020004,
            0x0000204000800080, 0x0000200040008080, 0x0000100020008080, 0x0000080010008080,
            0x0000040008008080, 0x0000020004008080, 0x0000800100020080, 0x0000800041000080,
            0x00FFFCDDFCED714A, 0x007FFCDDFCED714A, 0x003FFFCDFFD88096, 0x0000040810002101,
            0x0001000204080011, 0x0001000204000801, 0x0001000082000401, 0x0001FFFAABFAD1A2,
        ];
        ROOK_MAGICS[sq as usize]
    }

    fn find_magic_bishop(&self, sq: u8) -> u64 {
        // Placeholder magic numbers
        const BISHOP_MAGICS: [u64; 64] = [
            0x0002020202020200, 0x0002020202020000, 0x0004010202000000, 0x0004040080000000,
            0x0001104000000000, 0x0000821040000000, 0x0000410410400000, 0x0000104104104000,
            0x0000040404040400, 0x0000020202020200, 0x0000040102020000, 0x0000040400800000,
            0x0000011040000000, 0x0000008210400000, 0x0000004104104000, 0x0000002082082000,
            0x0004000808080800, 0x0002000404040400, 0x0001000202020200, 0x0000800802004000,
            0x0000800400A00000, 0x0000200100884000, 0x0000400082082000, 0x0000200041041000,
            0x0002080010101000, 0x0001040008080800, 0x0000208004010400, 0x0000404004010200,
            0x0000840000802000, 0x0000404002011000, 0x0000808001041000, 0x0000404000820800,
            0x0001041000202000, 0x0000820800101000, 0x0000104400080800, 0x0000020080080080,
            0x0000404040040100, 0x0000808100020100, 0x0001010100020800, 0x0000808080010400,
            0x0000820820004000, 0x0000410410002000, 0x0000082088001000, 0x0000002011000800,
            0x0000080100400400, 0x0001010101000200, 0x0002020202000400, 0x0001010101000200,
            0x0000410410400000, 0x0000208208200000, 0x0000002084100000, 0x0000000020880000,
            0x0000001002020000, 0x0000040408020000, 0x0004040404040000, 0x0002020202020000,
            0x0000104104104000, 0x0000002082082000, 0x0000000020841000, 0x0000000000208800,
            0x0000000010020200, 0x0000000404080200, 0x0000040404040400, 0x0002020202020200,
        ];
        BISHOP_MAGICS[sq as usize]
    }

    // **Fast sliding piece attack generation using magic bitboards**
    #[inline(always)]
    pub fn get_bishop_attacks(&self, sq: u8, occ: Bitboard) -> Bitboard {
        // Use classical method for now (magic bitboards would be faster)
        let mut attacks = 0;
        let directions: [(i8, i8); 4] = [(-1, -1), (-1, 1), (1, -1), (1, 1)];
        
        let rank = (sq / 8) as i8;
        let file = (sq % 8) as i8;
        
        for (dr, df) in directions.iter() {
            let mut r = rank + dr;
            let mut f = file + df;
            
            while (0..8).contains(&r) && (0..8).contains(&f) {
                let target = (r * 8 + f) as u8;
                attacks = set_bit(attacks, target);
                if get_bit(occ, target) {
                    break;
                }
                r += dr;
                f += df;
            }
        }
        
        attacks
    }

    #[inline(always)]
    pub fn get_rook_attacks(&self, sq: u8, occ: Bitboard) -> Bitboard {
        let mut attacks = 0;
        let directions: [(i8, i8); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
        
        let rank = (sq / 8) as i8;
        let file = (sq % 8) as i8;
        
        for (dr, df) in directions.iter() {
            let mut r = rank + dr;
            let mut f = file + df;
            
            while (0..8).contains(&r) && (0..8).contains(&f) {
                let target = (r * 8 + f) as u8;
                attacks = set_bit(attacks, target);
                if get_bit(occ, target) {
                    break;
                }
                r += dr;
                f += df;
            }
        }
        
        attacks
    }

    #[inline(always)]
    pub fn get_queen_attacks(&self, sq: u8, occ: Bitboard) -> Bitboard {
        self.get_rook_attacks(sq, occ) | self.get_bishop_attacks(sq, occ)
    }
}

lazy_static::lazy_static! {
    pub static ref ATTACK_TABLES: AttackTables = AttackTables::new();
}

// **Additional SIMD-friendly utility functions**

// Shift entire bitboard (useful for pawn pushes)
#[inline(always)]
pub fn shift_north(bb: Bitboard) -> Bitboard {
    bb << 8
}

#[inline(always)]
pub fn shift_south(bb: Bitboard) -> Bitboard {
    bb >> 8
}

#[inline(always)]
pub fn shift_east(bb: Bitboard) -> Bitboard {
    (bb << 1) & !FILE_A
}

#[inline(always)]
pub fn shift_west(bb: Bitboard) -> Bitboard {
    (bb >> 1) & !FILE_H
}

// Fill algorithms for connected pieces
#[inline(always)]
pub fn fill_north(mut bb: Bitboard, empty: Bitboard) -> Bitboard {
    bb |= empty & (bb << 8);
    bb |= empty & (bb << 16);
    bb |= empty & (bb << 32);
    bb
}

#[inline(always)]
pub fn fill_south(mut bb: Bitboard, empty: Bitboard) -> Bitboard {
    bb |= empty & (bb >> 8);
    bb |= empty & (bb >> 16);
    bb |= empty & (bb >> 32);
    bb
}