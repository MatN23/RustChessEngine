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

#[inline(always)]
pub fn pop_lsb(bb: Bitboard) -> (Bitboard, Option<u8>) {
    if bb == 0 {
        return (0, None);
    }
    let sq = bb.trailing_zeros() as u8;
    (bb & (bb - 1), Some(sq))
}

#[inline(always)]
pub fn lsb(bb: Bitboard) -> Option<u8> {
    if bb == 0 {
        None
    } else {
        Some(bb.trailing_zeros() as u8)
    }
}

#[inline(always)]
pub fn count_bits(bb: Bitboard) -> u32 {
    bb.count_ones()
}

pub struct AttackTables {
    pub pawn_attacks: [[Bitboard; 64]; 2],
    pub knight_attacks: [Bitboard; 64],
    pub king_attacks: [Bitboard; 64],
}

impl AttackTables {
    pub fn new() -> Self {
        let mut tables = AttackTables {
            pawn_attacks: [[0; 64]; 2],
            knight_attacks: [0; 64],
            king_attacks: [0; 64],
        };
        
        tables.init_pawn_attacks();
        tables.init_knight_attacks();
        tables.init_king_attacks();
        
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

    pub fn get_bishop_attacks(&self, sq: u8, occ: Bitboard) -> Bitboard {
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