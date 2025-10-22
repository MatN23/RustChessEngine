use crate::board::{BoardState, Piece, Color, PIECE_VALUES};
use crate::bitboard::*;

// ══════════════════════════════════════════════════════════════════════════════
// PROFESSIONAL EVALUATION WEIGHTS (Tournament Tuned)
// ══════════════════════════════════════════════════════════════════════════════

// Material (adjusted for better endgame scaling)
const PAWN_VALUE: i32 = 100;
const KNIGHT_VALUE: i32 = 320;
const BISHOP_VALUE: i32 = 330;
const ROOK_VALUE: i32 = 500;
const QUEEN_VALUE: i32 = 900;

// Tactical Safety Weights (CRITICAL FOR PREVENTING BLUNDERS)
const HANGING_PIECE_PENALTY: i32 = 150;  // Severe penalty for undefended pieces
const ABSOLUTE_PIN_PENALTY: i32 = 50;    // Severe penalty for absolute pins
const RELATIVE_PIN_PENALTY: i32 = 15;    // Lighter penalty for relative pins
const FORK_BONUS: i32 = 50;              // Bonus for creating forks
const SKEWER_BONUS: i32 = 40;            // Bonus for skewers
const DISCOVERED_ATTACK_BONUS: i32 = 35; // Bonus for discovered attacks
const TRAPPED_PIECE: i32 = 120;          // Heavy penalty for trapped pieces
const THREAT_BONUS: i32 = 30;            // Bonus for creating threats

// Positional Weights
const BISHOP_PAIR_BONUS: i32 = 50;
const ROOK_OPEN_FILE: i32 = 25;
const ROOK_SEMI_OPEN: i32 = 15;
const ROOK_SEVENTH_RANK: i32 = 20;
const CONNECTED_ROOKS: i32 = 15;
const KNIGHT_OUTPOST: i32 = 30;
const BISHOP_LONG_DIAGONAL: i32 = 20;
const BAD_BISHOP_PENALTY: i32 = 20;
const FIANCHETTO_BONUS: i32 = 15;

// Pawn Structure
const DOUBLED_PAWN: i32 = 15;
const ISOLATED_PAWN: i32 = 20;
const BACKWARD_PAWN: i32 = 12;
const PASSED_PAWN_BONUS: [i32; 8] = [0, 10, 20, 40, 70, 120, 200, 0];
const PROTECTED_PASSED_PAWN: [i32; 8] = [0, 5, 10, 20, 35, 60, 100, 0];
const CANDIDATE_PASSED: [i32; 8] = [0, 5, 8, 15, 25, 40, 70, 0];
const PAWN_CHAIN_BONUS: i32 = 8;
const PAWN_STORM_BONUS: i32 = 12;

// King Safety
const PAWN_SHIELD_BONUS: i32 = 15;
const OPEN_FILE_NEAR_KING: i32 = 20;
const KING_ZONE_ATTACK: i32 = 10;
const CASTLING_RIGHTS_BONUS: i32 = 25;
const KING_ATTACK_WEIGHT: [i32; 6] = [0, 0, 50, 75, 88, 94]; // By attacker count

// Space and Mobility
const SPACE_BONUS: i32 = 2;
const SAFE_MOBILITY_BONUS: i32 = 4;
const KNIGHT_MOBILITY: i32 = 4;
const BISHOP_MOBILITY: i32 = 3;
const ROOK_MOBILITY: i32 = 2;
const QUEEN_MOBILITY: i32 = 1;

// Tempo
const TEMPO_BONUS: i32 = 15;

// Piece-Square Tables (Enhanced with better positional understanding)
const PAWN_PST_MG: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,
     90,  95,  90,  85,  85,  90,  95,  90,
     40,  50,  60,  75,  75,  60,  50,  40,
     30,  40,  55,  70,  70,  55,  40,  30,
     20,  30,  45,  60,  60,  45,  30,  20,
     15,  20,  25,  40,  40,  25,  20,  15,
     10,  15, -5, -15, -15,  -5,  15,  10,
      0,   0,   0,   0,   0,   0,   0,   0
];

const PAWN_PST_EG: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,
    130, 130, 130, 130, 130, 130, 130, 130,
     90,  95, 100, 105, 105, 100,  95,  90,
     60,  65,  70,  80,  80,  70,  65,  60,
     35,  40,  45,  55,  55,  45,  40,  35,
     20,  25,  30,  35,  35,  30,  25,  20,
     10,  15,  15,  15,  15,  15,  15,  10,
      0,   0,   0,   0,   0,   0,   0,   0
];

const KNIGHT_PST_MG: [i32; 64] = [
    -70, -60, -50, -45, -45, -50, -60, -70,
    -60, -30,   0,  15,  15,   0, -30, -60,
    -50,   0,  25,  40,  40,  25,   0, -50,
    -45,  15,  35,  50,  50,  35,  15, -45,
    -45,  15,  35,  50,  50,  35,  15, -45,
    -50,   0,  30,  40,  40,  30,   0, -50,
    -60, -30,   0,  10,  10,   0, -30, -60,
    -70, -60, -50, -45, -45, -50, -60, -70
];

const KNIGHT_PST_EG: [i32; 64] = [
    -70, -60, -50, -45, -45, -50, -60, -70,
    -60, -40, -30, -25, -25, -30, -40, -60,
    -50, -30,   0,  15,  15,   0, -30, -50,
    -45, -25,  15,  25,  25,  15, -25, -45,
    -45, -25,  15,  25,  25,  15, -25, -45,
    -50, -30,   0,  15,  15,   0, -30, -50,
    -60, -40, -30, -25, -25, -30, -40, -60,
    -70, -60, -50, -45, -45, -50, -60, -70
];

const BISHOP_PST_MG: [i32; 64] = [
    -30, -20, -20, -20, -20, -20, -20, -30,
    -20,   0,   5,  10,  10,   5,   0, -20,
    -20,   5,  20,  30,  30,  20,   5, -20,
    -20,  10,  25,  40,  40,  25,  10, -20,
    -20,  10,  30,  45,  45,  30,  10, -20,
    -20,  20,  25,  30,  30,  25,  20, -20,
    -20,   5,   0,   0,   0,   0,   5, -20,
    -30, -20, -20, -20, -20, -20, -20, -30
];

const BISHOP_PST_EG: [i32; 64] = [
    -30, -20, -20, -20, -20, -20, -20, -30,
    -20,   0,   5,   5,   5,   5,   0, -20,
    -20,   5,  15,  20,  20,  15,   5, -20,
    -20,   5,  20,  25,  25,  20,   5, -20,
    -20,   5,  20,  25,  25,  20,   5, -20,
    -20,   5,  15,  20,  20,  15,   5, -20,
    -20,   0,   5,   5,   5,   5,   0, -20,
    -30, -20, -20, -20, -20, -20, -20, -30
];

const ROOK_PST_MG: [i32; 64] = [
      0,   0,   5,  10,  10,   5,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
     15,  20,  20,  20,  20,  20,  20,  15,
      5,   5,   5,  10,  10,   5,   5,   5
];

const ROOK_PST_EG: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0
];

const QUEEN_PST_MG: [i32; 64] = [
    -30, -20, -15, -10, -10, -15, -20, -30,
    -20, -10,   0,   5,   5,   0, -10, -20,
    -15,   0,   5,  10,  10,   5,   0, -15,
    -10,   5,  10,  15,  15,  10,   5, -10,
    -10,   5,  10,  15,  15,  10,   5, -10,
    -15,   0,   5,  10,  10,   5,   0, -15,
    -20, -10,   0,   0,   0,   0, -10, -20,
    -30, -20, -15, -10, -10, -15, -20, -30
];

const QUEEN_PST_EG: [i32; 64] = [
    -30, -20, -15, -10, -10, -15, -20, -30,
    -20, -10,   0,   5,   5,   0, -10, -20,
    -15,   0,  15,  20,  20,  15,   0, -15,
    -10,   5,  20,  25,  25,  20,   5, -10,
    -10,   5,  20,  25,  25,  20,   5, -10,
    -15,   0,  15,  20,  20,  15,   0, -15,
    -20, -10,   0,   5,   5,   0, -10, -20,
    -30, -20, -15, -10, -10, -15, -20, -30
];

const KING_PST_MG: [i32; 64] = [
    -70, -60, -60, -65, -65, -60, -60, -70,
    -60, -50, -50, -55, -55, -50, -50, -60,
    -50, -40, -40, -45, -45, -40, -40, -50,
    -40, -30, -30, -35, -35, -30, -30, -40,
    -30, -20, -20, -25, -25, -20, -20, -30,
    -20, -10, -10, -15, -15, -10, -10, -20,
      0,  10,  10,   0,   0,  10,  10,   0,
     25,  35,  20,   0,   0,  20,  35,  25
];

const KING_PST_EG: [i32; 64] = [
    -60, -50, -40, -30, -30, -40, -50, -60,
    -50, -40, -30, -20, -20, -30, -40, -50,
    -40, -30,  10,  20,  20,  10, -30, -40,
    -30, -20,  20,  30,  30,  20, -20, -30,
    -30, -20,  20,  30,  30,  20, -20, -30,
    -40, -30,  10,  20,  20,  10, -30, -40,
    -50, -40, -30, -20, -20, -30, -40, -50,
    -60, -50, -40, -30, -30, -40, -50, -60
];

// Move PinType outside of impl block
#[derive(PartialEq)]
enum PinType {
    None,
    Absolute,  // Pinned to king
    Relative,  // Pinned to valuable piece
}

pub struct Evaluator;

impl Evaluator {
    pub fn evaluate(board: &BoardState) -> i32 {
        // Quick draw detection
        if board.halfmove_clock >= 100 {
            return 0;
        }

        let phase = Self::game_phase(board);
        
        // Core evaluation components
        let (mg_score, eg_score) = Self::material_and_pst(board);
        let mut score = Self::tapered_eval(mg_score, eg_score, phase);

        // CRITICAL: Tactical safety (prevents blunders)
        score += Self::tactical_safety(board, phase);
        
        // Positional evaluation
        score += Self::pawn_structure(board, phase);
        score += Self::piece_mobility_safe(board, phase);
        score += Self::king_safety_advanced(board, phase);
        score += Self::space_evaluation(board, phase);
        score += Self::rook_evaluation(board);
        score += Self::bishop_evaluation(board);
        score += Self::knight_evaluation(board);
        score += Self::tempo_bonus(board);

        // Return from side-to-move perspective
        if board.side_to_move == Color::Black {
            -score
        } else {
            score
        }
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // TACTICAL SAFETY - PREVENTS BLUNDERS (HIGHEST PRIORITY)
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn tactical_safety(board: &BoardState, phase: i32) -> i32 {
        let mut score = 0;
        let tables = &ATTACK_TABLES;
        
        // Check both sides for hanging pieces and tactical threats
        for color in 0..2 {
            let sign = if color == 0 { 1 } else { -1 };
            let enemy_color = if color == 0 { 1 } else { 0 };
            
            // Check all pieces for being undefended or underdefended
            for piece_type in 1..=5 {  // Pawn to Queen (not King)
                let mut pieces = board.pieces[color][piece_type];
                
                while pieces != 0 {
                    let (new_bb, sq) = pop_lsb(pieces);
                    pieces = new_bb;
                    let square = sq.unwrap();
                    
                    // SEE (Static Exchange Evaluation) for this square
                    let see_score = Self::see_square(board, square, color as u8);
                    
                    if see_score < 0 {
                        // Losing the piece
                        score += sign * see_score;
                    }
                    
                    // Advanced pin detection
                    let pin_type = Self::detect_pin_type(board, square, color as u8, tables);
                    match pin_type {
                        PinType::Absolute => {
                            score += sign * -ABSOLUTE_PIN_PENALTY;
                        }
                        PinType::Relative => {
                            score += sign * -RELATIVE_PIN_PENALTY;
                        }
                        PinType::None => {}
                    }
                    
                    // Trapped piece detection
                    if Self::is_piece_trapped(board, square, piece_type, color, tables) {
                        score += sign * -TRAPPED_PIECE;
                    }
                }
            }
            
            // Threat detection (what can we attack next move?)
            score += sign * Self::detect_threats(board, color as u8, tables);
            
            // Fork detection (knight and pawn forks)
            score += sign * Self::detect_forks(board, color as u8, tables);
            
            // Skewer detection
            score += sign * Self::detect_skewers(board, color as u8, tables);
            
            // Discovered attack potential
            score += sign * Self::detect_discovered_attacks(board, color as u8, tables);
        }
        
        // Scale tactical awareness by game phase (more critical in middlegame)
        (score * (12 + phase)) / 24
    }

    // 🎯 SEE - Static Exchange Evaluation
    fn see_square(board: &BoardState, square: u8, defender_color: u8) -> i32 {
        let attacker_color = 1 - defender_color;
        let tables = &ATTACK_TABLES;
        
        // Get piece on square
        let (piece, _) = match board.piece_at(square) {
            Some(p) => p,
            None => return 0,
        };
        
        let target_value = PIECE_VALUES[piece as usize];
        
        // Find least valuable attacker
        let attacker_sq = Self::find_least_valuable_attacker(board, square, attacker_color as usize, tables);
        
        if attacker_sq.is_none() {
            // No attackers, piece is safe
            return 0;
        }
        
        let attacker_sq = attacker_sq.unwrap();
        let (attacker_piece, _) = board.piece_at(attacker_sq).unwrap();
        let attacker_value = PIECE_VALUES[attacker_piece as usize];
        
        // Simple SEE: if we lose more than we gain, it's bad
        let gain = target_value;
        
        // Check if piece is defended
        let defenders = Self::count_attackers(board, square, defender_color as usize, tables);
        
        if defenders == 0 {
            return -target_value; // Hanging piece
        }
        
        // Approximate exchange
        let loss = attacker_value.min(target_value);
        
        if gain > loss + 100 {
            return 0; // Winning capture
        } else if gain < loss - 100 {
            return gain - loss; // Losing
        }
        
        0 // About equal
    }

    fn find_least_valuable_attacker(board: &BoardState, square: u8, color: usize, tables: &AttackTables) -> Option<u8> {
        // Check in order: Pawn, Knight, Bishop, Rook, Queen, King
        for piece_type in 1..=6 {
            let pieces = board.pieces[color][piece_type];
            
            let attackers = match piece_type {
                1 => tables.pawn_attacks[1 - color][square as usize] & pieces,
                2 => tables.knight_attacks[square as usize] & pieces,
                3 => tables.get_bishop_attacks(square, board.all_pieces) & pieces,
                4 => tables.get_rook_attacks(square, board.all_pieces) & pieces,
                5 => tables.get_queen_attacks(square, board.all_pieces) & pieces,
                6 => tables.king_attacks[square as usize] & pieces,
                _ => 0,
            };
            
            if attackers != 0 {
                return lsb(attackers);
            }
        }
        
        None
    }

    // 🔍 Advanced Pin Detection
    fn detect_pin_type(board: &BoardState, square: u8, color: u8, tables: &AttackTables) -> PinType {
        let king_sq = match board.get_king_square(if color == 0 { Color::White } else { Color::Black }) {
            Some(sq) => sq,
            None => return PinType::None,
        };
        
        let enemy_color = if color == 0 { 1 } else { 0 };
        let square_bb = 1u64 << square;
        
        // Check diagonal pins
        let bishop_attacks_from_king = tables.get_bishop_attacks(king_sq, board.all_pieces);
        if (bishop_attacks_from_king & square_bb) != 0 {
            let occ_without_piece = board.all_pieces & !(square_bb);
            let extended_attacks = tables.get_bishop_attacks(king_sq, occ_without_piece);
            if (extended_attacks & (board.pieces[enemy_color][Piece::Bishop as usize] |
                                   board.pieces[enemy_color][Piece::Queen as usize])) != 0 {
                return PinType::Absolute;
            }
        }
        
        // Check file/rank pins
        let rook_attacks_from_king = tables.get_rook_attacks(king_sq, board.all_pieces);
        if (rook_attacks_from_king & square_bb) != 0 {
            let occ_without_piece = board.all_pieces & !(square_bb);
            let extended_attacks = tables.get_rook_attacks(king_sq, occ_without_piece);
            if (extended_attacks & (board.pieces[enemy_color][Piece::Rook as usize] |
                                   board.pieces[enemy_color][Piece::Queen as usize])) != 0 {
                return PinType::Absolute;
            }
        }
        
        // Check relative pins (pinned to queen or rook)
        for piece_type in 4..=5 {  // Rook and Queen
            let mut valuable_pieces = board.pieces[color as usize][piece_type];
            
            while valuable_pieces != 0 {
                let (new_bb, valuable_sq) = pop_lsb(valuable_pieces);
                valuable_pieces = new_bb;
                let target_sq = valuable_sq.unwrap();
                
                if target_sq == square {
                    continue;
                }
                
                // Check if our piece is between enemy attacker and our valuable piece
                let attacks = if piece_type == 4 {
                    tables.get_rook_attacks(target_sq, board.all_pieces)
                } else {
                    tables.get_queen_attacks(target_sq, board.all_pieces)
                };
                
                if (attacks & square_bb) != 0 {
                    let occ_without = board.all_pieces & !(square_bb);
                    let extended = if piece_type == 4 {
                        tables.get_rook_attacks(target_sq, occ_without)
                    } else {
                        tables.get_queen_attacks(target_sq, occ_without)
                    };
                    
                    if (extended & (board.pieces[enemy_color][Piece::Rook as usize] |
                                   board.pieces[enemy_color][Piece::Queen as usize])) != 0 {
                        return PinType::Relative;
                    }
                }
            }
        }
        
        PinType::None
    }

    // ⚡ Trapped Piece Detection
    fn is_piece_trapped(board: &BoardState, square: u8, piece_type: usize, color: usize, tables: &AttackTables) -> bool {
        if piece_type == 1 || piece_type == 6 {
            return false; // Don't check pawns or king
        }
        
        let moves = match piece_type {
            2 => tables.knight_attacks[square as usize],
            3 => tables.get_bishop_attacks(square, board.all_pieces),
            4 => tables.get_rook_attacks(square, board.all_pieces),
            5 => tables.get_queen_attacks(square, board.all_pieces),
            _ => return false,
        };
        
        let safe_moves = moves & !board.color_bb[color];
        let enemy_color = 1 - color;
        
        // Count safe squares (not attacked by enemy)
        let mut safe_count = 0;
        let mut temp_moves = safe_moves;
        
        while temp_moves != 0 {
            let (new_bb, sq) = pop_lsb(temp_moves);
            temp_moves = new_bb;
            let target = sq.unwrap();
            
            if !board.is_square_attacked(target, if enemy_color == 0 { Color::White } else { Color::Black }) {
                safe_count += 1;
            }
        }
        
        // If fewer than 2 safe moves, piece is likely trapped
        safe_count < 2
    }

    // 🛡️ Threat Detection
    fn detect_threats(board: &BoardState, color: u8, tables: &AttackTables) -> i32 {
        let mut threat_score = 0;
        let enemy_color = 1 - color as usize;
        
        // Check what we can attack on next move
        for piece_type in 1..=6 {
            let mut pieces = board.pieces[color as usize][piece_type];
            
            while pieces != 0 {
                let (new_bb, sq) = pop_lsb(pieces);
                pieces = new_bb;
                let square = sq.unwrap();
                
                let attacks = match piece_type {
                    1 => tables.pawn_attacks[color as usize][square as usize],
                    2 => tables.knight_attacks[square as usize],
                    3 => tables.get_bishop_attacks(square, board.all_pieces),
                    4 => tables.get_rook_attacks(square, board.all_pieces),
                    5 => tables.get_queen_attacks(square, board.all_pieces),
                    6 => tables.king_attacks[square as usize],
                    _ => 0,
                };
                
                // Count valuable enemy pieces we're attacking
                let threatened_pieces = attacks & board.color_bb[enemy_color];
                let mut temp = threatened_pieces;
                
                while temp != 0 {
                    let (new_t, t_sq) = pop_lsb(temp);
                    temp = new_t;
                    let target = t_sq.unwrap();
                    
                    if let Some((t_piece, _)) = board.piece_at(target) {
                        let t_value = PIECE_VALUES[t_piece as usize];
                        let our_value = PIECE_VALUES[piece_type];
                        
                        // Bonus if we attack more valuable piece
                        if t_value > our_value + 100 {
                            threat_score += THREAT_BONUS;
                        }
                    }
                }
            }
        }
        
        threat_score
    }
    
    fn count_attackers(board: &BoardState, square: u8, color: usize, tables: &AttackTables) -> i32 {
        let mut count = 0;
        
        // Pawn attackers
        let pawn_attacks = tables.pawn_attacks[1 - color][square as usize];
        count += count_bits(pawn_attacks & board.pieces[color][Piece::Pawn as usize]) as i32;
        
        // Knight attackers
        let knight_attacks = tables.knight_attacks[square as usize];
        count += count_bits(knight_attacks & board.pieces[color][Piece::Knight as usize]) as i32;
        
        // Bishop/Queen diagonal attackers
        let bishop_attacks = tables.get_bishop_attacks(square, board.all_pieces);
        count += count_bits(bishop_attacks & (board.pieces[color][Piece::Bishop as usize] |
                                              board.pieces[color][Piece::Queen as usize])) as i32;
        
        // Rook/Queen file attackers
        let rook_attacks = tables.get_rook_attacks(square, board.all_pieces);
        count += count_bits(rook_attacks & (board.pieces[color][Piece::Rook as usize] |
                                            board.pieces[color][Piece::Queen as usize])) as i32;
        
        // King attackers
        let king_attacks = tables.king_attacks[square as usize];
        count += count_bits(king_attacks & board.pieces[color][Piece::King as usize]) as i32;
        
        count
    }
    
    fn detect_forks(board: &BoardState, color: u8, tables: &AttackTables) -> i32 {
        let mut fork_score = 0;
        let enemy_color = 1 - color as usize;
        
        // Knight forks
        let mut knights = board.pieces[color as usize][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            
            let attacks = tables.knight_attacks[square as usize];
            let attacked_pieces = attacks & board.color_bb[enemy_color];
            
            if count_bits(attacked_pieces) >= 2 {
                fork_score += FORK_BONUS;
            }
        }
        
        // Pawn forks
        let mut pawns = board.pieces[color as usize][Piece::Pawn as usize];
        while pawns != 0 {
            let (new_bb, sq) = pop_lsb(pawns);
            pawns = new_bb;
            let square = sq.unwrap();
            
            let attacks = tables.pawn_attacks[color as usize][square as usize];
            let attacked_pieces = attacks & board.color_bb[enemy_color];
            
            if count_bits(attacked_pieces) >= 2 {
                fork_score += FORK_BONUS / 2;
            }
        }
        
        fork_score
    }

    // 🎨 Skewer Detection
    fn detect_skewers(board: &BoardState, color: u8, tables: &AttackTables) -> i32 {
        let mut skewer_score = 0;
        let enemy_color = 1 - color as usize;
        
        // Check bishops and queens for skewers
        for piece_type in [Piece::Bishop as usize, Piece::Queen as usize] {
            let mut pieces = board.pieces[color as usize][piece_type];
            
            while pieces != 0 {
                let (new_bb, sq) = pop_lsb(pieces);
                pieces = new_bb;
                let square = sq.unwrap();
                
                let attacks = if piece_type == Piece::Bishop as usize {
                    tables.get_bishop_attacks(square, board.all_pieces)
                } else {
                    tables.get_bishop_attacks(square, board.all_pieces) | 
                    tables.get_rook_attacks(square, board.all_pieces)
                };
                
                // Look for valuable piece with less valuable piece behind it
                let mut temp = attacks & board.color_bb[enemy_color];
                while temp != 0 {
                    let (new_t, t_sq) = pop_lsb(temp);
                    temp = new_t;
                    let target = t_sq.unwrap();
                    
                    if let Some((front_piece, _)) = board.piece_at(target) {
                        // Remove front piece and see what's behind
                        let occ_without = board.all_pieces & !(1u64 << target);
                        let extended = if piece_type == Piece::Bishop as usize {
                            tables.get_bishop_attacks(square, occ_without)
                        } else {
                            tables.get_queen_attacks(square, occ_without)
                        };
                        
                        let behind = extended & board.color_bb[enemy_color] & !(attacks);
                        if behind != 0 {
                            if let Some((back_piece, _)) = board.piece_at(lsb(behind).unwrap()) {
                                let front_val = PIECE_VALUES[front_piece as usize];
                                let back_val = PIECE_VALUES[back_piece as usize];
                                
                                // Skewer if front piece is more valuable
                                if front_val >= back_val + 200 {
                                    skewer_score += SKEWER_BONUS;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        skewer_score
    }

    // 🌟 Discovered Attack Detection
    fn detect_discovered_attacks(board: &BoardState, color: u8, tables: &AttackTables) -> i32 {
        let mut discovered_score = 0;
        let enemy_color = 1 - color as usize;
        
        // Check for potential discovered attacks by moving pieces
        for piece_type in 1..=6 {
            let mut pieces = board.pieces[color as usize][piece_type];
            
            while pieces != 0 {
                let (new_bb, sq) = pop_lsb(pieces);
                pieces = new_bb;
                let square = sq.unwrap();
                let square_bb = 1u64 << square;
                
                // Check what's behind this piece
                for attacker_type in [Piece::Bishop as usize, Piece::Rook as usize, Piece::Queen as usize] {
                    let mut attackers = board.pieces[color as usize][attacker_type];
                    
                    while attackers != 0 {
                        let (new_a, a_sq) = pop_lsb(attackers);
                        attackers = new_a;
                        let attacker_sq = a_sq.unwrap();
                        
                        if attacker_sq == square {
                            continue;
                        }
                        
                        let attacks = match attacker_type {
                            3 => tables.get_bishop_attacks(attacker_sq, board.all_pieces),
                            4 => tables.get_rook_attacks(attacker_sq, board.all_pieces),
                            5 => tables.get_queen_attacks(attacker_sq, board.all_pieces),
                            _ => 0,
                        };
                        
                        // If our piece is on the attack line
                        if (attacks & square_bb) != 0 {
                            // See what would be attacked if piece moves
                            let occ_without = board.all_pieces & !square_bb;
                            let extended = match attacker_type {
                                3 => tables.get_bishop_attacks(attacker_sq, occ_without),
                                4 => tables.get_rook_attacks(attacker_sq, occ_without),
                                5 => tables.get_queen_attacks(attacker_sq, occ_without),
                                _ => 0,
                            };
                            
                            let new_targets = extended & board.color_bb[enemy_color] & !attacks;
                            if new_targets != 0 {
                                discovered_score += DISCOVERED_ATTACK_BONUS;
                            }
                        }
                    }
                }
            }
        }
        
        discovered_score
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // GAME PHASE AND TAPERING
    // ══════════════════════════════════════════════════════════════════════════════
    
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

    fn tapered_eval(mg_score: i32, eg_score: i32, phase: i32) -> i32 {
        (mg_score * phase + eg_score * (24 - phase)) / 24
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // MATERIAL AND PIECE-SQUARE TABLES
    // ══════════════════════════════════════════════════════════════════════════════
    
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
                    
                    let (mg_bonus, eg_bonus) = match piece_type {
                        1 => (PAWN_PST_MG[pst_sq as usize], PAWN_PST_EG[pst_sq as usize]),
                        2 => (KNIGHT_PST_MG[pst_sq as usize], KNIGHT_PST_EG[pst_sq as usize]),
                        3 => (BISHOP_PST_MG[pst_sq as usize], BISHOP_PST_EG[pst_sq as usize]),
                        4 => (ROOK_PST_MG[pst_sq as usize], ROOK_PST_EG[pst_sq as usize]),
                        5 => (QUEEN_PST_MG[pst_sq as usize], QUEEN_PST_EG[pst_sq as usize]),
                        6 => (KING_PST_MG[pst_sq as usize], KING_PST_EG[pst_sq as usize]),
                        _ => (0, 0),
                    };
                    
                    mg_score += sign * mg_bonus;
                    eg_score += sign * eg_bonus;
                }
            }
        }

        (mg_score, eg_score)
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // PAWN STRUCTURE
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn pawn_structure(board: &BoardState, phase: i32) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // Doubled, isolated, backward pawns
        for file in 0..8 {
            let file_mask = FILE_A << file;
            
            // White doubled pawns
            let white_on_file = count_bits(white_pawns & file_mask);
            if white_on_file > 1 {
                score -= DOUBLED_PAWN * (white_on_file - 1) as i32;
            }

            // Black doubled pawns
            let black_on_file = count_bits(black_pawns & file_mask);
            if black_on_file > 1 {
                score += DOUBLED_PAWN * (black_on_file - 1) as i32;
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
                score -= ISOLATED_PAWN;
            }

            if (black_pawns & file_mask) != 0 && (black_pawns & adjacent_files) == 0 {
                score += ISOLATED_PAWN;
            }
        }

        // Passed pawns
        score += Self::passed_pawn_evaluation(board, phase);
        
        // Pawn chains
        score += Self::pawn_chains(board, phase);
        
        // Pawn storms
        score += Self::pawn_storms(board, phase);

        score
    }

    // 🏰 Pawn Storm Evaluation
    fn pawn_storms(board: &BoardState, phase: i32) -> i32 {
        let mut score = 0;
        
        // Only relevant in middlegame with opposite side castling
        if phase < 12 {
            return 0;
        }
        
        let white_king_sq = board.get_king_square(Color::White);
        let black_king_sq = board.get_king_square(Color::Black);
        
        if white_king_sq.is_none() || black_king_sq.is_none() {
            return 0;
        }
        
        let wk_sq = white_king_sq.unwrap();
        let bk_sq = black_king_sq.unwrap();
        
        let wk_file = wk_sq % 8;
        let bk_file = bk_sq % 8;
        
        // White pawn storm against black king
        let mut white_pawns = board.pieces[0][Piece::Pawn as usize];
        while white_pawns != 0 {
            let (new_bb, sq) = pop_lsb(white_pawns);
            white_pawns = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;
            
            // Bonus for advancing pawns near enemy king
            if (file as i32 - bk_file as i32).abs() <= 1 && rank >= 4 {
                score += PAWN_STORM_BONUS * (rank as i32 - 3);
            }
        }
        
        // Black pawn storm against white king
        let mut black_pawns = board.pieces[1][Piece::Pawn as usize];
        while black_pawns != 0 {
            let (new_bb, sq) = pop_lsb(black_pawns);
            black_pawns = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;
            
            if (file as i32 - wk_file as i32).abs() <= 1 && rank <= 3 {
                score -= PAWN_STORM_BONUS * (4 - rank as i32);
            }
        }
        
        score
    }

    fn passed_pawn_evaluation(board: &BoardState, phase: i32) -> i32 {
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
                let mut bonus = PASSED_PAWN_BONUS[rank as usize];
                
                // Protected passed pawn
                let protection_mask = if file > 0 { 1u64 << (square - 9) } else { 0 } |
                                     if file < 7 { 1u64 << (square - 7) } else { 0 };
                if (white_pawns & protection_mask) != 0 {
                    bonus += PROTECTED_PASSED_PAWN[rank as usize];
                }
                
                // King proximity (more important in endgame)
                if let Some(king_sq) = board.get_king_square(Color::White) {
                    let king_dist = ((king_sq / 8) as i32 - rank as i32).abs() + 
                                   ((king_sq % 8) as i32 - file as i32).abs();
                    bonus += ((8 - king_dist) * (24 - phase)) / 8;
                }
                
                // Enemy king distance (penalty if enemy king is close)
                if let Some(enemy_king_sq) = board.get_king_square(Color::Black) {
                    let enemy_king_dist = ((enemy_king_sq / 8) as i32 - rank as i32).abs() + 
                                         ((enemy_king_sq % 8) as i32 - file as i32).abs();
                    bonus -= ((8 - enemy_king_dist) * (24 - phase)) / 12;
                }
                
                score += bonus;
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
                let mut bonus = PASSED_PAWN_BONUS[(7 - rank) as usize];
                
                let protection_mask = if file > 0 { 1u64 << (square + 7) } else { 0 } |
                                     if file < 7 { 1u64 << (square + 9) } else { 0 };
                if (black_pawns & protection_mask) != 0 {
                    bonus += PROTECTED_PASSED_PAWN[(7 - rank) as usize];
                }
                
                if let Some(king_sq) = board.get_king_square(Color::Black) {
                    let king_dist = ((king_sq / 8) as i32 - rank as i32).abs() + 
                                   ((king_sq % 8) as i32 - file as i32).abs();
                    bonus += ((8 - king_dist) * (24 - phase)) / 8;
                }
                
                if let Some(enemy_king_sq) = board.get_king_square(Color::White) {
                    let enemy_king_dist = ((enemy_king_sq / 8) as i32 - rank as i32).abs() + 
                                         ((enemy_king_sq % 8) as i32 - file as i32).abs();
                    bonus -= ((8 - enemy_king_dist) * (24 - phase)) / 12;
                }
                
                score -= bonus;
            }
        }

        score
    }
    
    fn pawn_chains(board: &BoardState, phase: i32) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White king safety
        if let Some(king_sq) = board.get_king_square(Color::White) {
            let mut safety = 0;
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            // Pawn shield
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in 1..=2 {
                        let r = king_rank as i8 + dr;
                        if r < 8 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(white_pawns, sq) {
                                safety += PAWN_SHIELD_BONUS * (3 - dr as i32);
                            } else {
                                safety -= PAWN_SHIELD_BONUS / 2;
                            }
                        }
                    }
                }
            }

            // Open files near king
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    let file_mask = FILE_A << f;
                    if (white_pawns & file_mask) == 0 {
                        safety -= OPEN_FILE_NEAR_KING;
                        // Extra penalty if enemy rooks/queens on the file
                        if (board.pieces[1][Piece::Rook as usize] & file_mask) != 0 ||
                           (board.pieces[1][Piece::Queen as usize] & file_mask) != 0 {
                            safety -= OPEN_FILE_NEAR_KING;
                        }
                    }
                }
            }

            // Attack pattern recognition
            let attackers = Self::count_king_zone_attackers(board, king_sq, Color::Black);
            if attackers > 0 {
                let attack_index = (attackers as usize).min(5);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            // Penalty for king in center during middlegame
            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank <= 2 {
                safety -= 30;
            }

            // Castling rights bonus
            if board.castling_rights & 3 != 0 {
                safety += CASTLING_RIGHTS_BONUS;
            }

            score += (safety * phase) / 24;
        }

        // Black king safety
        if let Some(king_sq) = board.get_king_square(Color::Black) {
            let mut safety = 0;
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in -2..=-1 {
                        let r = king_rank as i8 + dr;
                        if r >= 0 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(black_pawns, sq) {
                                safety += PAWN_SHIELD_BONUS * (3 + dr as i32);
                            } else {
                                safety -= PAWN_SHIELD_BONUS / 2;
                            }
                        }
                    }
                }
            }

            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    let file_mask = FILE_A << f;
                    if (black_pawns & file_mask) == 0 {
                        safety -= OPEN_FILE_NEAR_KING;
                        if (board.pieces[0][Piece::Rook as usize] & file_mask) != 0 ||
                           (board.pieces[0][Piece::Queen as usize] & file_mask) != 0 {
                            safety -= OPEN_FILE_NEAR_KING;
                        }
                    }
                }
            }

            let attackers = Self::count_king_zone_attackers(board, king_sq, Color::White);
            if attackers > 0 {
                let attack_index = (attackers as usize).min(5);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank >= 5 {
                safety -= 30;
            }

            if board.castling_rights & 12 != 0 {
                safety += CASTLING_RIGHTS_BONUS;
            }

            score -= (safety * phase) / 24;
        }

        score
    }

    fn count_king_zone_attackers(board: &BoardState, king_sq: u8, by_color: Color) -> i32 {
        let tables = &ATTACK_TABLES;
        let king_zone = tables.king_attacks[king_sq as usize] | (1u64 << king_sq);
        let color = by_color as usize;
        
        let mut attackers = 0;
        
        // Count different piece types attacking king zone
        for piece_type in 2..=5 {  // Knight to Queen
            let mut pieces = board.pieces[color][piece_type];
            
            while pieces != 0 {
                let (new_bb, sq) = pop_lsb(pieces);
                pieces = new_bb;
                let square = sq.unwrap();
                
                let attacks = match piece_type {
                    2 => tables.knight_attacks[square as usize],
                    3 => tables.get_bishop_attacks(square, board.all_pieces),
                    4 => tables.get_rook_attacks(square, board.all_pieces),
                    5 => tables.get_queen_attacks(square, board.all_pieces),
                    _ => 0,
                };
                
                if (attacks & king_zone) != 0 {
                    attackers += 1;
                }
            }
        }
        
        attackers
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // PIECE-SPECIFIC EVALUATIONS
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn rook_evaluation(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White rooks
        let mut rooks = board.pieces[0][Piece::Rook as usize];
        let mut white_rook_files = Vec::new();
        
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;
            let file_mask = FILE_A << file;

            white_rook_files.push(file);

            // Open file
            if (white_pawns & file_mask) == 0 && (black_pawns & file_mask) == 0 {
                score += ROOK_OPEN_FILE;
            } 
            // Semi-open file
            else if (white_pawns & file_mask) == 0 {
                score += ROOK_SEMI_OPEN;
            }

            // 7th rank bonus
            if rank == 6 {
                score += ROOK_SEVENTH_RANK;
                if let Some(enemy_king) = board.get_king_square(Color::Black) {
                    if enemy_king / 8 == 7 {
                        score += ROOK_SEVENTH_RANK;
                    }
                }
            }
        }

        // Connected rooks
        if white_rook_files.len() == 2 {
            if white_rook_files[0].abs_diff(white_rook_files[1]) == 1 {
                score += CONNECTED_ROOKS;
            }
        }

        // Black rooks
        let mut rooks = board.pieces[1][Piece::Rook as usize];
        let mut black_rook_files = Vec::new();
        
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let file = square % 8;
            let rank = square / 8;
            let file_mask = FILE_A << file;

            black_rook_files.push(file);

            if (white_pawns & file_mask) == 0 && (black_pawns & file_mask) == 0 {
                score -= ROOK_OPEN_FILE;
            } else if (black_pawns & file_mask) == 0 {
                score -= ROOK_SEMI_OPEN;
            }

            if rank == 1 {
                score -= ROOK_SEVENTH_RANK;
                if let Some(enemy_king) = board.get_king_square(Color::White) {
                    if enemy_king / 8 == 0 {
                        score -= ROOK_SEVENTH_RANK;
                    }
                }
            }
        }

        if black_rook_files.len() == 2 {
            if black_rook_files[0].abs_diff(black_rook_files[1]) == 1 {
                score -= CONNECTED_ROOKS;
            }
        }

        score
    }

    fn bishop_evaluation(board: &BoardState) -> i32 {
        let mut score = 0;
        
        // Bishop pair bonus
        let white_bishops = count_bits(board.pieces[0][Piece::Bishop as usize]);
        let black_bishops = count_bits(board.pieces[1][Piece::Bishop as usize]);

        if white_bishops >= 2 {
            score += BISHOP_PAIR_BONUS;
        }
        if black_bishops >= 2 {
            score -= BISHOP_PAIR_BONUS;
        }

        // Bad bishop detection
        score += Self::bad_bishop_penalty(board);
        
        // Fianchetto patterns
        score += Self::fianchetto_patterns(board);

        score
    }
    
    // 🎨 Fianchetto Pattern Recognition
    fn fianchetto_patterns(board: &BoardState) -> i32 {
        let mut score = 0;
        
        // White fianchetto squares: b2, g2
        let white_bishops = board.pieces[0][Piece::Bishop as usize];
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        
        // Kingside fianchetto (g2 bishop, f3, g3, h3 pawns)
        if get_bit(white_bishops, 6) {  // g1 becomes 6 in our notation
            let supporting_pawns = get_bit(white_pawns, 14) || get_bit(white_pawns, 15);  // f2, g2
            if supporting_pawns {
                score += FIANCHETTO_BONUS;
            }
        }
        
        // Queenside fianchetto (b2 bishop)
        if get_bit(white_bishops, 1) {
            let supporting_pawns = get_bit(white_pawns, 9) || get_bit(white_pawns, 10);  // a2, b2
            if supporting_pawns {
                score += FIANCHETTO_BONUS;
            }
        }
        
        // Black fianchetto
        let black_bishops = board.pieces[1][Piece::Bishop as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];
        
        // Kingside (g7)
        if get_bit(black_bishops, 62) {
            let supporting_pawns = get_bit(black_pawns, 54) || get_bit(black_pawns, 55);
            if supporting_pawns {
                score -= FIANCHETTO_BONUS;
            }
        }
        
        // Queenside (b7)
        if get_bit(black_bishops, 57) {
            let supporting_pawns = get_bit(black_pawns, 49) || get_bit(black_pawns, 50);
            if supporting_pawns {
                score -= FIANCHETTO_BONUS;
            }
        }
        
        score
    }
    
    fn bad_bishop_penalty(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];
        
        // White bishops
        let mut bishops = board.pieces[0][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            
            let is_light_square = (square / 8 + square % 8) % 2 == 0;
            
            let mut blocked_count = 0;
            let mut temp_pawns = white_pawns;
            while temp_pawns != 0 {
                let (new_p, p_sq) = pop_lsb(temp_pawns);
                temp_pawns = new_p;
                let pawn_square = p_sq.unwrap();
                let pawn_is_light = (pawn_square / 8 + pawn_square % 8) % 2 == 0;
                
                if pawn_is_light == is_light_square && pawn_square / 8 >= 3 {
                    blocked_count += 1;
                }
            }
            
            if blocked_count >= 4 {
                score -= BAD_BISHOP_PENALTY;
            }
        }
        
        // Black bishops
        let mut bishops = board.pieces[1][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            
            let is_light_square = (square / 8 + square % 8) % 2 == 0;
            
            let mut blocked_count = 0;
            let mut temp_pawns = black_pawns;
            while temp_pawns != 0 {
                let (new_p, p_sq) = pop_lsb(temp_pawns);
                temp_pawns = new_p;
                let pawn_square = p_sq.unwrap();
                let pawn_is_light = (pawn_square / 8 + pawn_square % 8) % 2 == 0;
                
                if pawn_is_light == is_light_square && pawn_square / 8 <= 4 {
                    blocked_count += 1;
                }
            }
            
            if blocked_count >= 4 {
                score += BAD_BISHOP_PENALTY;
            }
        }
        
        score
    }

    fn knight_evaluation(board: &BoardState) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White knight outposts
        let mut knights = board.pieces[0][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;

            if rank >= 4 {
                let mut protected = false;
                if file > 0 && square >= 9 && get_bit(white_pawns, square - 9) {
                    protected = true;
                }
                if file < 7 && square >= 7 && get_bit(white_pawns, square - 7) {
                    protected = true;
                }
                
                if protected {
                    score += KNIGHT_OUTPOST;
                    
                    let mut can_be_attacked = false;
                    if file > 0 {
                        let ahead_file = FILE_A << (file - 1);
                        if (black_pawns & ahead_file) != 0 {
                            can_be_attacked = true;
                        }
                    }
                    if file < 7 {
                        let ahead_file = FILE_A << (file + 1);
                        if (black_pawns & ahead_file) != 0 {
                            can_be_attacked = true;
                        }
                    }
                    
                    if !can_be_attacked {
                        score += KNIGHT_OUTPOST / 2;
                    }
                }
            }
        }

        // Black knight outposts
        let mut knights = board.pieces[1][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let rank = square / 8;
            let file = square % 8;

            if rank <= 3 {
                let mut protected = false;
                if file > 0 && square < 56 && get_bit(black_pawns, square + 7) {
                    protected = true;
                }
                if file < 7 && square < 56 && get_bit(black_pawns, square + 9) {
                    protected = true;
                }
                
                if protected {
                    score -= KNIGHT_OUTPOST;
                    
                    let mut can_be_attacked = false;
                    if file > 0 {
                        let ahead_file = FILE_A << (file - 1);
                        if (white_pawns & ahead_file) != 0 {
                            can_be_attacked = true;
                        }
                    }
                    if file < 7 {
                        let ahead_file = FILE_A << (file + 1);
                        if (white_pawns & ahead_file) != 0 {
                            can_be_attacked = true;
                        }
                    }
                    
                    if !can_be_attacked {
                        score -= KNIGHT_OUTPOST / 2;
                    }
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

    // ══════════════════════════════════════════════════════════════════════════════
    // SAFE MOBILITY (ONLY COUNT SAFE SQUARES)
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn piece_mobility_safe(board: &BoardState, phase: i32) -> i32 {
        let mut white_mobility = 0;
        let mut black_mobility = 0;
        let tables = &ATTACK_TABLES;

        // Build enemy attack maps
        let white_attacks = build_attack_map(board, 0, tables);
        let black_attacks = build_attack_map(board, 1, tables);

        // Knights - safe mobility
        let mut knights = board.pieces[0][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let attacks = tables.knight_attacks[square as usize] & !board.color_bb[0];
            let safe_attacks = attacks & !black_attacks;
            white_mobility += count_bits(safe_attacks) as i32 * KNIGHT_MOBILITY;
        }

        let mut knights = board.pieces[1][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let attacks = tables.knight_attacks[square as usize] & !board.color_bb[1];
            let safe_attacks = attacks & !white_attacks;
            black_mobility += count_bits(safe_attacks) as i32 * KNIGHT_MOBILITY;
        }

        // Bishops - safe mobility
        let mut bishops = board.pieces[0][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[0];
            let safe_attacks = attacks & !black_attacks;
            white_mobility += count_bits(safe_attacks) as i32 * BISHOP_MOBILITY;
        }

        let mut bishops = board.pieces[1][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[1];
            let safe_attacks = attacks & !white_attacks;
            black_mobility += count_bits(safe_attacks) as i32 * BISHOP_MOBILITY;
        }

        // Rooks - safe mobility
        let mut rooks = board.pieces[0][Piece::Rook as usize];
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_rook_attacks(square, board.all_pieces) & !board.color_bb[0];
            let safe_attacks = attacks & !black_attacks;
            white_mobility += count_bits(safe_attacks) as i32 * ROOK_MOBILITY;
        }

        let mut rooks = board.pieces[1][Piece::Rook as usize];
        while rooks != 0 {
            let (new_bb, sq) = pop_lsb(rooks);
            rooks = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_rook_attacks(square, board.all_pieces) & !board.color_bb[1];
            let safe_attacks = attacks & !white_attacks;
            black_mobility += count_bits(safe_attacks) as i32 * ROOK_MOBILITY;
        }

        // Queens - safe mobility
        let mut queens = board.pieces[0][Piece::Queen as usize];
        while queens != 0 {
            let (new_bb, sq) = pop_lsb(queens);
            queens = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_queen_attacks(square, board.all_pieces) & !board.color_bb[0];
            let safe_attacks = attacks & !black_attacks;
            white_mobility += count_bits(safe_attacks) as i32 * QUEEN_MOBILITY;
        }

        let mut queens = board.pieces[1][Piece::Queen as usize];
        while queens != 0 {
            let (new_bb, sq) = pop_lsb(queens);
            queens = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_queen_attacks(square, board.all_pieces) & !board.color_bb[1];
            let safe_attacks = attacks & !white_attacks;
            black_mobility += count_bits(safe_attacks) as i32 * QUEEN_MOBILITY;
        }

        ((white_mobility - black_mobility) * phase) / 24
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // SPACE EVALUATION
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn space_evaluation(board: &BoardState, phase: i32) -> i32 {
        // Space matters more in middlegame
        if phase < 12 {
            return 0;
        }
        
        // Define center and extended center
        const CENTER: Bitboard = 0x0000001818000000; // e4,d4,e5,d5
        const EXTENDED_CENTER: Bitboard = 0x00003C3C3C3C0000; // Ranks 3-6, files c-f
        
        let white_control = build_attack_map(board, 0, &ATTACK_TABLES);
        let black_control = build_attack_map(board, 1, &ATTACK_TABLES);
        
        let white_center = count_bits(white_control & CENTER) as i32;
        let black_center = count_bits(black_control & CENTER) as i32;
        
        let white_extended = count_bits(white_control & EXTENDED_CENTER) as i32;
        let black_extended = count_bits(black_control & EXTENDED_CENTER) as i32;
        
        let center_score = (white_center - black_center) * SPACE_BONUS * 2;
        let extended_score = (white_extended - black_extended) * SPACE_BONUS;
        
        ((center_score + extended_score) * phase) / 24
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // ELITE KING SAFETY
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn king_safety_advanced(board: &BoardState, phase: i32) -> i32 {
        // King safety mainly matters in middlegame
        if phase < 10 {
            return 0;
        }

        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White king safety
        if let Some(king_sq) = board.get_king_square(Color::White) {
            let mut safety = 0;
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            // Pawn shield
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in 1..=2 {
                        let r = king_rank as i8 + dr;
                        if r < 8 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(white_pawns, sq) {
                                safety += PAWN_SHIELD_BONUS * (3 - dr as i32);
                            } else {
                                safety -= PAWN_SHIELD_BONUS / 2;
                            }
                        }
                    }
                }
            }

            // Open files near king
            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    let file_mask = FILE_A << f;
                    if (white_pawns & file_mask) == 0 {
                        safety -= OPEN_FILE_NEAR_KING;
                        // Extra penalty if enemy rooks/queens on the file
                        if (board.pieces[1][Piece::Rook as usize] & file_mask) != 0 ||
                            (board.pieces[1][Piece::Queen as usize] & file_mask) != 0 {
                            safety -= OPEN_FILE_NEAR_KING;
                        }
                    }
                }
            }

            // Attack pattern recognition
            let attackers = Self::count_king_zone_attackers(board, king_sq, Color::Black);
            if attackers > 0 {
                let attack_index = (attackers as usize).min(5);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            // Penalty for king in center during middlegame
            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank <= 2 {
                safety -= 30;
            }

            // Castling rights bonus
            if board.castling_rights & 3 != 0 {
                safety += CASTLING_RIGHTS_BONUS;
            }

            score += (safety * phase) / 24;
        }

        // Black king safety
        if let Some(king_sq) = board.get_king_square(Color::Black) {
            let mut safety = 0;
            let king_file = king_sq % 8;
            let king_rank = king_sq / 8;

            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    for dr in -2..=-1 {
                        let r = king_rank as i8 + dr;
                        if r >= 0 {
                            let sq = (r * 8 + f) as u8;
                            if get_bit(black_pawns, sq) {
                                safety += PAWN_SHIELD_BONUS * (3 + dr as i32);
                            } else {
                                safety -= PAWN_SHIELD_BONUS / 2;
                            }
                        }
                    }
                }
            }

            for df in -1..=1 {
                let f = king_file as i8 + df;
                if f >= 0 && f < 8 {
                    let file_mask = FILE_A << f;
                    if (black_pawns & file_mask) == 0 {
                        safety -= OPEN_FILE_NEAR_KING;
                        if (board.pieces[0][Piece::Rook as usize] & file_mask) != 0 ||
                            (board.pieces[0][Piece::Queen as usize] & file_mask) != 0 {
                            safety -= OPEN_FILE_NEAR_KING;
                        }
                    }
                }
            }

            let attackers = Self::count_king_zone_attackers(board, king_sq, Color::White);
            if attackers > 0 {
                let attack_index = (attackers as usize).min(5);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank >= 5 {
                safety -= 30;
            }

            if board.castling_rights & 12 != 0 {
                safety += CASTLING_RIGHTS_BONUS;
            }

            score -= (safety * phase) / 24;
        }

        score
    }
}

// Helper function moved outside impl block
fn build_attack_map(board: &BoardState, color: usize, tables: &AttackTables) -> Bitboard {
    let mut attacks = 0u64;
    
    // Pawn attacks
    let mut pawns = board.pieces[color][Piece::Pawn as usize];
    while pawns != 0 {
        let (new_bb, sq) = pop_lsb(pawns);
        pawns = new_bb;
        let square = sq.unwrap();
        attacks |= tables.pawn_attacks[color][square as usize];
    }
    
    // Knight attacks
    let mut knights = board.pieces[color][Piece::Knight as usize];
    while knights != 0 {
        let (new_bb, sq) = pop_lsb(knights);
        knights = new_bb;
        let square = sq.unwrap();
        attacks |= tables.knight_attacks[square as usize];
    }
    
    // Bishop attacks
    let mut bishops = board.pieces[color][Piece::Bishop as usize];
    while bishops != 0 {
        let (new_bb, sq) = pop_lsb(bishops);
        bishops = new_bb;
        let square = sq.unwrap();
        attacks |= tables.get_bishop_attacks(square, board.all_pieces);
    }
    
    // Rook attacks
    let mut rooks = board.pieces[color][Piece::Rook as usize];
    while rooks != 0 {
        let (new_bb, sq) = pop_lsb(rooks);
        rooks = new_bb;
        let square = sq.unwrap();
        attacks |= tables.get_rook_attacks(square, board.all_pieces);
    }
    
    // Queen attacks
    let mut queens = board.pieces[color][Piece::Queen as usize];
    while queens != 0 {
        let (new_bb, sq) = pop_lsb(queens);
        queens = new_bb;
        let square = sq.unwrap();
        attacks |= tables.get_queen_attacks(square, board.all_pieces);
    }
    
    // King attacks
    let king = board.pieces[color][Piece::King as usize];
    if king != 0 {
        let square = lsb(king).unwrap();
        attacks |= tables.king_attacks[square as usize];
    }
    
    attacks
}