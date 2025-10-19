use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;

pub struct Zobrist {
    pub piece_keys: [[[u64; 64]; 7]; 2], // [color][piece][square]
    pub castle_keys: [u64; 16],           // 16 possible castling states
    pub ep_keys: [u64; 8],                // 8 files for en passant
    pub side_key: u64,                    // Side to move
}

impl Zobrist {
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(42);
        
        let mut piece_keys = [[[0u64; 64]; 7]; 2];
        for color in 0..2 {
            for piece in 0..7 {
                for square in 0..64 {
                    piece_keys[color][piece][square] = rng.gen();
                }
            }
        }

        let mut castle_keys = [0u64; 16];
        for i in 0..16 {
            castle_keys[i] = rng.gen();
        }

        let mut ep_keys = [0u64; 8];
        for i in 0..8 {
            ep_keys[i] = rng.gen();
        }

        let side_key = rng.gen();

        Zobrist {
            piece_keys,
            castle_keys,
            ep_keys,
            side_key,
        }
    }

    pub fn hash_piece(&self, color: usize, piece: usize, square: usize) -> u64 {
        self.piece_keys[color][piece][square]
    }

    pub fn hash_castling(&self, rights: u8) -> u64 {
        self.castle_keys[rights as usize]
    }

    pub fn hash_ep(&self, file: u8) -> u64 {
        self.ep_keys[file as usize]
    }

    pub fn hash_side(&self) -> u64 {
        self.side_key
    }
}

lazy_static::lazy_static! {
    pub static ref ZOBRIST: Zobrist = Zobrist::new();
}