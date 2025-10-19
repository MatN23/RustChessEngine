use crate::board::{BoardState, Piece, Color, PIECE_VALUES};
use crate::bitboard::*;

// Evaluation weights
const BISHOP_PAIR_BONUS: i32 = 50;
const ROOK_OPEN_FILE_BONUS: i32 = 25;
const ROOK_SEMI_OPEN_FILE_BONUS: i32 = 10;
const KNIGHT_OUTPOST_BONUS: i32 = 30;
const DOUBLED_PAWN_PENALTY: i32 = 15;
const ISOLATED_PAWN_PENALTY: i32 = 20;
const TEMPO_BONUS: i32 = 15;

const PASSED_PAWN_BONUS: [i32; 8] = [0, 10, 20, 35, 60, 100, 170, 0];

// Piece-Square Tables
const PAWN_PST_MG: [i32; 64] = [
    10, 10, 10, 10, 10, 10, 10, 10,
    70, 70, 60, 60, 60, 60, 70, 70,
    30, 40, 50, 60, 60, 50, 40, 30,
    20, 30, 45, 60, 60, 45, 30, 20,
    10, 20, 35, 50, 50, 35, 20, 10,
     5,  0, -5, 20, 20, -5,  0,  5,
     5, 15, 15,-15,-15, 15, 15,  5,
     0,  0,  0,  0,  0,  0,  0,  0
];

const KNIGHT_PST_MG: [i32; 64] = [
   -50,-40,-30,-30,-30,-30,-40,-50,
   -40,-15,  5, 15, 15,  5,-15,-40,
   -30,  5, 25, 40, 40, 25,  5,-30,
   -30, 10, 30, 45, 45, 30, 10,-30,
   -30, 10, 30, 45, 45, 30, 10,-30,
   -30,  5, 25, 40, 40, 25,  5,-30,
   -40,-15,  5, 15, 15,  5,-15,-40,
   -50,-40,-30,-30,-30,-30,-40,-50
];

const BISHOP_PST_MG: [i32; 64] = [
   -20,-10,-10,-10,-10,-10,-10,-20,
   -10,  0,  0, 15, 15,  0,  0,-10,
   -10,  0, 20, 30, 30, 20,  0,-10,
   -10, 10, 25, 40, 40, 25, 10,-10,
   -10, 10, 30, 45, 45, 30, 10,-10,
   -10, 15, 25, 30, 30, 25, 15,-10,
   -10,  5,  0,  0,  0,  0,  5,-10,
   -20,-10,-10,-10,-10,-10,-10,-20
];

const ROOK_PST_MG: [i32; 64] = [
    0,  0,  0, 20, 20, 0,  0, 0,
   -5,  0,  0, 15, 15, 0,  0,-5,
   -5,  0,  0, 15, 15, 0,  0,-5,
   -5,  0,  0, 15, 15, 0,  0,-5,
   -5,  0,  0, 15, 15, 0,  0,-5,
   -5,  0,  0, 15, 15, 0,  0,-5,
    5, 20, 20, 30, 30, 20, 20, 5,
    0,  0,  0, 20, 20, 0,  0, 0
];

const QUEEN_PST_MG: [i32; 64] = [
   -30,-20,-10,-10,-10,-10,-20,-30,
   -20,-10,  0,  0,  0,  0,-10,-20,
   -10,  0,  5,  5,  5,  5,  0,-10,
    -5,  0, 10, 15, 15, 10,  0, -5,
    -5,  0, 10, 15, 15, 10,  0, -5,
   -10,  0,  5,  5,  5,  5,  0,-10,
   -20,-10,  0,  0,  0,  0,-10,-20,
   -30,-20,-10,-10,-10,-10,-20,-30
];

const KING_PST_MG: [i32; 64] = [
   -50,-40,-40,-40,-40,-40,-40,-50,
   -40,-30,-30,-30,-30,-30,-30,-40,
   -30,-20,-20,-20,-20,-20,-20,-30,
   -20,-10,-10,-10,-10,-10,-10,-20,
   -10,  0,  0,  0,  0,  0,  0,-10,
     0,  5,  5,  5,  5,  5,  5,  0,
    10, 10, 10, 10, 10, 10, 10, 10,
    20, 20, 10,  0,  0, 10, 20, 20
];

const KING_PST_EG: [i32; 64] = [
   -40,-30,-20,-10,-10,-20,-30,-40,
   -30,-20,-10,  0,  0,-10,-20,-30,
   -20,-10, 10, 20, 20, 10,-10,-20,
   -10,  0, 20, 30, 30, 20,  0,-10,
   -10,  0, 20, 30, 30, 20,  0,-10,
   -20,-10, 10, 20, 20, 10,-10,-20,
   -30,-20,-10,  0,  0,-10,-20,-30,
   -40,-30,-20,-10,-10,-20,-30,-40
];

pub struct Evaluator;

impl Evaluator {
    pub fn evaluate(board: &BoardState) -> i32 {
        if board.halfmove_clock >= 100 {
            return 0; // Draw by 50-move rule
        }

        let phase = Self::game_phase(board);
        
        let (mg_score, eg_score) = Self::material_and_pst(board);
        let mut score = (mg_score * phase + eg_score * (24 - phase)) / 24;

        // Positional bonuses
        score += Self::central_control_bonus(board);
        score += Self::bishop_pair_bonus(board);
        score += Self::rook_bonuses(board);
        score += Self::mobility_bonus(board, phase);
        score += Self::pawn_structure_bonus(board);
        score += Self::passed_pawn_bonus(board);
        score += Self::king_safety_bonus(board, phase);
        score += Self::piece_coordination(board);
        score += Self::tempo_bonus(board);

        // Return from side to move perspective
        if board.side_to_move == Color::Black {
            -score
        } else {
            score
        }
    }

    fn game_phase(board: &BoardState) -> i32 {
        let mut phase = 0;
        phase += count_bits(board.pieces[0][Piece::Knight as usize]) as i32;
        phase += count_bits(board.pieces[1][Piece::Knight as usize]) as i32;
        phase += count_bits(board.pieces[0][Piece::Bishop as usize]) as i32;
        phase += count_bits(board.pieces[1][Piece::Bishop as usize]) as i32;
        phase += count_bits(board.pieces[0][Piece::Rook as usize]) as i32 * 2;
        phase += count_bits(board.pieces[1][Piece::Rook as usize]) as i32 * 2;
        phase += count_bits(board.pieces[0][Piece::Queen as usize]) as i32 * 4;
        phase += count_bits(board.pieces[1][Piece::Queen as usize]) as i32 * 4;
        phase.min(24)
    }

    fn material_and_pst(board: &BoardState) -> (i32, i32) {
        let mut mg_score = 0;
        let mut eg_score = 0;

        for color in 0..2 {
            let sign = if color == 0 { 1 } else { -1 };

            for piece_type in 1..=6 {
                let pieces = board.pieces[color][piece_type];
                let count = count_bits(pieces) as i32;
                let material = PIECE_VALUES[piece_type] * count;
                mg_score += sign * material;
                eg_score += sign * material;

                let mut temp = pieces;
                while temp != 0 {
                    let (new_bb, sq) = pop_lsb(temp);
                    temp = new_bb;
                    let square = sq.unwrap();
                    
                    let pst_sq = if color == 0 { square } else { square ^ 56 };
                    
                    let mg_bonus = match piece_type {
                        1 => PAWN_PST_MG[pst_sq as usize],
                        2 => KNIGHT_PST_MG[pst_sq as usize],
                        3 => BISHOP_PST_MG[pst_sq as usize],
                        4 => ROOK_PST_MG[pst_sq as usize],
                        5 => QUEEN_PST_MG[pst_sq as usize],
                        6 => KING_PST_MG[pst_sq as usize],
                        _ => 0,
                    };
                    
                    let eg_bonus = if piece_type == 6 {
                        KING_PST_EG[pst_sq as usize]
                    } else {
                        mg_bonus
                    };
                    
                    mg_score += sign * mg_bonus;
                    eg_score += sign * eg_bonus;
                }
            }
        }

        (mg_score, eg_score)
    }

    fn central_control_bonus(board: &BoardState) -> i32 {
        let mut score = 0;
        let center_squares = [27, 28, 35, 36];
        let extended_center = [19, 20, 21, 26, 29, 34, 37, 42, 43, 44];

        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        for &sq in &center_squares {
            if get_bit(white_pawns, sq) {
                score += 25;
            }
            if get_bit(black_pawns, sq) {
                score -= 25;
            }
        }

        for &sq in &extended_center {
            if get_bit(white_pawns, sq) {
                score += 10;
            }
            if get_bit(black_pawns, sq) {
                score -= 10;
            }
        }

        score
    }

    fn bishop_pair_bonus(board: &BoardState) -> i32 {
        let mut score = 0;
        
        let white_bishops = count_bits(board.pieces[0][Piece::Bishop as usize]);
        let black_bishops = count_bits(board.pieces[1][Piece::Bishop as usize]);

        if white_bishops >= 2 {
            score += BISHOP_PAIR_BONUS;
        }
        if black_bishops >= 2 {
            score -= BISHOP_PAIR_BONUS;
        }

        score
    }

    fn rook_bonuses(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White rooks
        let mut rooks = board.pieces[0][Piece::Rook as usize];
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;
            let file_mask = FILE_A << file;

            if (white_pawns & file_mask) == 0 && (black_pawns & file_mask) == 0 {
                score += ROOK_OPEN_FILE_BONUS;
            } else if (white_pawns & file_mask) == 0 {
                score += ROOK_SEMI_OPEN_FILE_BONUS;
            }

            if rank == 6 {
                score += 20; // 7th rank bonus
            }
        }

        // Black rooks
        let mut rooks = board.pieces[1][Piece::Rook as usize];
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;
            let file_mask = FILE_A << file;

            if (white_pawns & file_mask) == 0 && (black_pawns & file_mask) == 0 {
                score -= ROOK_OPEN_FILE_BONUS;
            } else if (black_pawns & file_mask) == 0 {
                score -= ROOK_SEMI_OPEN_FILE_BONUS;
            }

            if rank == 1 {
                score -= 20; // 2nd rank bonus
            }
        }

        score
    }

    fn mobility_bonus(board: &BoardState, phase: i32) -> i32 {
        let mut white_mobility = 0;
        let mut black_mobility = 0;
        let mobility_weight = (phase / 3).max(1);
        let tables = &ATTACK_TABLES;

        // Knights
        let mut knights = board.pieces[0][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let attacks = tables.knight_attacks[square as usize] & !board.color_bb[0];
            white_mobility += count_bits(attacks) as i32;
        }

        let mut knights = board.pieces[1][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let attacks = tables.knight_attacks[square as usize] & !board.color_bb[1];
            black_mobility += count_bits(attacks) as i32;
        }

        // Bishops
        let mut bishops = board.pieces[0][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[0];
            white_mobility += count_bits(attacks) as i32;
        }

        let mut bishops = board.pieces[1][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[1];
            black_mobility += count_bits(attacks) as i32;
        }

        (white_mobility - black_mobility) * mobility_weight
    }

    fn pawn_structure_bonus(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // Check for doubled and isolated pawns
        for file in 0..8 {
            let file_mask = FILE_A << file;
            
            let white_on_file = count_bits(white_pawns & file_mask);
            if white_on_file > 1 {
                score -= DOUBLED_PAWN_PENALTY * (white_on_file - 1) as i32;
            }

            let black_on_file = count_bits(black_pawns & file_mask);
            if black_on_file > 1 {
                score += DOUBLED_PAWN_PENALTY * (black_on_file - 1) as i32;
            }

            // Isolated pawns
            let mut adjacent_files = 0u64;
            if file > 0 {
                adjacent_files |= FILE_A << (file - 1);
            }
            if file < 7 {
                adjacent_files |= FILE_A << (file + 1);
            }

            if (white_pawns & file_mask) != 0 && (white_pawns & adjacent_files) == 0 {
                score -= ISOLATED_PAWN_PENALTY;
            }

            if (black_pawns & file_mask) != 0 && (black_pawns & adjacent_files) == 0 {
                score += ISOLATED_PAWN_PENALTY;
            }
        }

        score
    }

    fn passed_pawn_bonus(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White passed pawns
        let mut temp = white_pawns;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;

            let mut ahead_mask = 0u64;
            for r in (rank + 1)..8 {
                if file > 0 {
                    ahead_mask = set_bit(ahead_mask, r * 8 + file - 1);
                }
                ahead_mask = set_bit(ahead_mask, r * 8 + file);
                if file < 7 {
                    ahead_mask = set_bit(ahead_mask, r * 8 + file + 1);
                }
            }

            if (black_pawns & ahead_mask) == 0 {
                score += PASSED_PAWN_BONUS[rank as usize];
            }
        }

        // Black passed pawns
        let mut temp = black_pawns;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;

            let mut ahead_mask = 0u64;
            for r in (0..rank).rev() {
                if file > 0 {
                    ahead_mask = set_bit(ahead_mask, r * 8 + file - 1);
                }
                ahead_mask = set_bit(ahead_mask, r * 8 + file);
                if file < 7 {
                    ahead_mask = set_bit(ahead_mask, r * 8 + file + 1);
                }
            }

            if (white_pawns & ahead_mask) == 0 {
                score -= PASSED_PAWN_BONUS[(6 - rank) as usize];
            }
        }

        score
    }

    fn king_safety_bonus(board: &BoardState, phase: i32) -> i32 {
        if phase < 8 {
            return 0; // Only relevant in middlegame/endgame
        }

        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White king safety
        if let Some(king_sq) = board.get_king_square(Color::White) {
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            let mut safety = 0;
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in 1..=2 {
                        let r = king_rank as i8 + dr;
                        if r < 8 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(white_pawns, sq) {
                                safety += if dr == 1 { 15 } else { 10 };
                            }
                        }
                    }
                }
            }

            if king_rank > 3 {
                safety -= 20; // King advanced too far
            }

            score += safety;
        }

        // Black king safety
        if let Some(king_sq) = board.get_king_square(Color::Black) {
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            let mut safety = 0;
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in -2..=-1 {
                        let r = king_rank as i8 + dr;
                        if r >= 0 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(black_pawns, sq) {
                                safety += if dr == -1 { 15 } else { 10 };
                            }
                        }
                    }
                }
            }

            if king_rank < 4 {
                safety -= 20;
            }

            score -= safety;
        }

        (score * phase) / 24
    }

    fn piece_coordination(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_knights = board.pieces[0][Piece::Knight as usize];
        let black_knights = board.pieces[1][Piece::Knight as usize];
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White knight outposts
        let mut temp = white_knights;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;

            if rank >= 4 {
                if file > 0 && get_bit(white_pawns, square - 9) {
                    score += KNIGHT_OUTPOST_BONUS;
                }
                if file < 7 && get_bit(white_pawns, square - 7) {
                    score += KNIGHT_OUTPOST_BONUS;
                }
            }
        }

        // Black knight outposts
        let mut temp = black_knights;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;

            if rank <= 3 {
                if file > 0 && get_bit(black_pawns, square + 7) {
                    score -= KNIGHT_OUTPOST_BONUS;
                }
                if file < 7 && get_bit(black_pawns, square + 9) {
                    score -= KNIGHT_OUTPOST_BONUS;
                }
            }
        }

        score
    }

    fn tempo_bonus(board: &BoardState) -> i32 {
        if board.side_to_move == Color::White {
            TEMPO_BONUS
        } else {
            -TEMPO_BONUS
        }
    }
}