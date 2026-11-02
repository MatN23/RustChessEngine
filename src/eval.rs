use crate::board::{BoardState, Piece, Color, PIECE_VALUES};
use crate::bitboard::*;

// ══════════════════════════════════════════════════════════════════════════════
// PROFESSIONAL EVALUATION WEIGHTS (Stockfish-Calibrated)
// ══════════════════════════════════════════════════════════════════════════════

// Material (tournament-tested values)
const PAWN_VALUE: i32 = 100;
const KNIGHT_VALUE: i32 = 325;
const BISHOP_VALUE: i32 = 335;
const ROOK_VALUE: i32 = 500;
const QUEEN_VALUE: i32 = 975;

// Tactical Safety Weights (CRITICAL FOR PREVENTING BLUNDERS)
const HANGING_PIECE_PENALTY: i32 = 80;   // Safer penalty (less than pawn value)
const ABSOLUTE_PIN_PENALTY: i32 = 65;    // Heavy penalty for absolute pins
const RELATIVE_PIN_PENALTY: i32 = 25;    // Lighter penalty for relative pins
const FORK_BONUS: i32 = 60;              // Bonus for creating forks
const SKEWER_BONUS: i32 = 45;            // Bonus for skewers
const DISCOVERED_ATTACK_BONUS: i32 = 40; // Bonus for discovered attacks
const TRAPPED_PIECE: i32 = 180;          // Heavy penalty for trapped pieces
const THREAT_BONUS: i32 = 35;            // Bonus for creating threats


// Positional Weights
const BISHOP_PAIR_BONUS: i32 = 55;
const ROOK_OPEN_FILE: i32 = 35;
const ROOK_SEMI_OPEN: i32 = 20;
const ROOK_SEVENTH_RANK: i32 = 30;
const CONNECTED_ROOKS: i32 = 20;
const KNIGHT_OUTPOST: i32 = 40;
const BISHOP_LONG_DIAGONAL: i32 = 25;
const BAD_BISHOP_PENALTY: i32 = 25;
const FIANCHETTO_BONUS: i32 = 18;

// Pawn Structure (significantly heavier penalties)
const DOUBLED_PAWN: i32 = 25;
const ISOLATED_PAWN: i32 = 30;
const BACKWARD_PAWN: i32 = 20;
const PASSED_PAWN_BONUS: [i32; 8] = [0, 15, 30, 60, 110, 180, 300, 0];
const PROTECTED_PASSED_PAWN: [i32; 8] = [0, 8, 15, 30, 55, 90, 150, 0];
const CANDIDATE_PASSED: [i32; 8] = [0, 8, 12, 22, 38, 60, 100, 0];
const PAWN_CHAIN_BONUS: i32 = 10;
const PAWN_STORM_BONUS: i32 = 15;

// King Safety (much stronger)
const PAWN_SHIELD_BONUS: i32 = 20;
const OPEN_FILE_NEAR_KING: i32 = 35;
const KING_ZONE_ATTACK: i32 = 15;
const CASTLING_RIGHTS_BONUS: i32 = 35;
// Quadratic king attack table — much more aggressive to prevent saving minor pieces at cost of king
const KING_ATTACK_WEIGHT: [i32; 8] = [0, 20, 100, 250, 450, 700, 950, 1200];


// Space and Mobility (much stronger mobility)
const SPACE_BONUS: i32 = 3;
const SAFE_MOBILITY_BONUS: i32 = 5;
const KNIGHT_MOBILITY: i32 = 7;
const BISHOP_MOBILITY: i32 = 5;
const ROOK_MOBILITY: i32 = 4;
const QUEEN_MOBILITY: i32 = 2;

// Tempo
const TEMPO_BONUS: i32 = 20;

// Connectivity (pieces protecting each other)
const CONNECTIVITY_BONUS: i32 = 4;

// Piece-Square Tables (Stockfish-Calibrated)
// Board layout: a1=0, b1=1, ..., h1=7, a2=8, ..., h8=63
// Tables are from White's perspective and flipped for Black

const PAWN_PST_MG: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,  // rank 1 (impossible)
     98, 134,  61,  95,  68, 126,  34, -11,  // rank 7
     -6,   7,  26,  31,  65,  56,  25, -20,  // rank 6
    -14,  13,   6,  21,  23,  12,  17, -23,  // rank 5
    -27,  -2,  -5,  12,  17,   6,  10, -25,  // rank 4
    -26,  -4,  -4, -10,   3,   3,  33, -12,  // rank 3
    -35,  -1, -20, -23, -15,  24,  38, -22,  // rank 2
      0,   0,   0,   0,   0,   0,   0,   0,  // rank 1 (impossible)
];

const PAWN_PST_EG: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,
    178, 173, 158, 134, 147, 132, 165, 187,
     94, 100,  85,  67,  56,  53,  82,  84,
     32,  24,  13,   5,  -2,   4,  17,  17,
     13,   9,  -3,  -7,  -7,  -8,   3,  -1,
      4,   7,  -6,   1,   0,  -5,  -1,  -8,
     13,   8,   8,  10,  13,   0,   2,  -7,
      0,   0,   0,   0,   0,   0,   0,   0,
];

const KNIGHT_PST_MG: [i32; 64] = [
   -167, -89, -34, -49,  61, -97, -15,-107,
    -73, -41,  72,  36,  23,  62,   7, -17,
    -47,  60,  37,  65,  84, 129,  73,  44,
     -9,  17,  19,  53,  37,  69,  18,  22,
    -13,   4,  16,  13,  28,  19,  21,  -8,
    -23,  -9,  12,  10,  19,  17,  25, -16,
    -29, -53, -12,  -3,  -1,  18, -14, -19,
   -105, -21, -58, -33, -17, -28, -19, -23,
];

const KNIGHT_PST_EG: [i32; 64] = [
    -58, -38, -13, -28, -31, -27, -63, -99,
    -25,  -8, -25,  -2,  -9, -25, -24, -52,
    -24, -20,  10,   9,  -1,  -9, -19, -41,
    -17,   3,  22,  22,  22,  11,   8, -18,
    -18,  -6,  16,  25,  16,  17,   4, -18,
    -23,  -3,  -1,  15,  10,  -3, -20, -22,
    -42, -20, -10,  -5,  -2, -20, -23, -44,
    -29, -51, -23, -15, -22, -18, -50, -64,
];

const BISHOP_PST_MG: [i32; 64] = [
    -29,   4, -82, -37, -25, -42,   7,  -8,
    -26,  16, -18, -13,  30,  59,  18, -47,
    -16,  37,  43,  40,  35,  50,  37,  -2,
     -4,   5,  19,  50,  37,  37,   7,  -2,
     -6,  13,  13,  26,  34,  12,  10,   4,
      0,  15,  15,  15,  14,  27,  18,  10,
      4,  15,  16,   0,   7,  21,  33,   1,
    -33,  -3, -14, -21, -13, -12, -39, -21,
];

const BISHOP_PST_EG: [i32; 64] = [
    -14, -21, -11,  -8,  -7,  -9, -17, -24,
     -8,  -4,   7, -12,  -3, -13,  -4, -14,
      2,  -8,   0,  -1,  -2,   6,   0,   4,
     -3,   9,  12,   9,  14,  10,   3,   2,
     -6,   3,  13,  19,   7,  10,  -3,  -9,
    -12,  -3,   8,  10,  13,   3,  -7, -15,
    -14, -18,  -7,  -1,   4,  -9, -15, -27,
    -23,  -9, -23,  -5,  -9, -16,  -5, -17,
];

const ROOK_PST_MG: [i32; 64] = [
     32,  42,  32,  51,  63,   9,  31,  43,
     27,  32,  58,  62,  80,  67,  26,  44,
     -5,  19,  26,  36,  17,  45,  61,  16,
    -24, -11,   7,  26,  24,  35,  -8, -20,
    -36, -26, -12,  -1,   9,  -7,   6, -23,
    -45, -25, -16, -17,   3,   0,  -5, -33,
    -44, -16, -20,  -9,  -1,  11,  -6, -71,
    -19, -13,   1,  17,  16,   7, -37, -26,
];

const ROOK_PST_EG: [i32; 64] = [
     13,  10,  18,  15,  12,  12,   8,   5,
     11,  13,  13,  11,  -3,   3,   8,   3,
      7,   7,   7,   5,   4,  -3,  -5,  -3,
      4,   3,  13,   1,   2,   1,  -1,   2,
      3,   5,   8,   4,  -5,  -6,  -8, -11,
     -4,   0,  -5,  -1,  -7, -12,  -8, -16,
     -6,  -6,   0,   2,  -9,  -9, -11,  -3,
     -9,   2,   3,  -1,  -5, -13,   4, -20,
];

const QUEEN_PST_MG: [i32; 64] = [
    -28,   0,  29,  12,  59,  44,  43,  45,
    -24, -39,  -5,   1, -16,  57,  28,  54,
    -13, -17,   7,   8,  29,  56,  47,  57,
    -27, -27, -16, -16,  -1,  17,  -2,   1,
     -9, -26,  -9, -10,  -2,  -4,   3,  -3,
    -14,   2, -11,  -2,  -5,   2,  14,   5,
    -35,  -8,  11,   2,   8,  15,  -3,   1,
     -1, -18,  -9,  10, -15, -25, -31, -50,
];

const QUEEN_PST_EG: [i32; 64] = [
     -9,  22,  22,  27,  27,  19,  10,  20,
    -17,  20,  32,  41,  58,  25,  30,   0,
    -20,   6,   9,  49,  47,  35,  19,   9,
      3,  22,  24,  45,  57,  40,  57,  36,
    -18,  28,  19,  47,  31,  34,  39,  23,
    -16, -27,  15,   6,   9,  17,  10,   5,
    -22, -23, -30, -16, -16, -23, -36, -32,
    -33, -28, -22, -43,  -5, -32, -20, -41,
];

const KING_PST_MG: [i32; 64] = [
    -65,  23,  16, -15, -56, -34,   2,  13,
     29,  -1, -20,  -7,  -8,  -4, -38, -29,
     -9,  24,   2, -16, -20,   6,  22, -22,
    -17, -20, -12, -27, -30, -25, -14, -36,
    -49,  -1, -27, -39, -46, -44, -33, -51,
    -14, -14, -22, -46, -44, -30, -15, -27,
      1,   7,  -8, -64, -43, -16,   9,   8,
    -15,  36,  12, -54,   8, -28,  24,  14,
];

const KING_PST_EG: [i32; 64] = [
    -74, -35, -18, -18, -11,  15,   4, -17,
    -12,  17,  14,  17,  17,  38,  23,  11,
     10,  17,  23,  15,  20,  45,  44,  13,
     -8,  22,  24,  27,  26,  33,  26,   3,
    -18,  -4,  21,  24,  27,  23,   9, -11,
    -19,  -3,  11,  21,  23,  16,   7,  -9,
    -27, -11,   4,  13,  14,   4,  -5, -17,
    -53, -34, -21, -11, -28, -14, -24, -43,
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
        // We now evaluate tactical safety statically but with tuned weights.
        // Tactical resolution is further handled by Quiescence Search and SEE.
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
        score += Self::piece_connectivity(board, phase);

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
        
        // Build attack maps once for the entire board
        let white_attacks = build_attack_map(board, 0, tables);
        let black_attacks = build_attack_map(board, 1, tables);
        
        // Check both sides for hanging pieces and tactical threats
        for color in 0..2 {
            let sign = if color == 0 { 1 } else { -1 };
            let _enemy_color = if color == 0 { 1 } else { 0 };
            
            let our_attacks = if color == 0 { white_attacks } else { black_attacks };
            let enemy_attacks = if color == 0 { black_attacks } else { white_attacks };
            
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
                        // Losing the piece - heavy penalty
                        score += sign * see_score;
                        
                        // Additional penalty if piece is completely undefended (hanging)
                        let is_defended = (our_attacks & (1u64 << square)) != 0;
                        if !is_defended {
                            score += sign * -HANGING_PIECE_PENALTY;
                        }
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
                    
                    // Trapped piece detection - pass attack maps to consider trade options
                    if Self::is_piece_trapped(board, square, piece_type, color, tables, enemy_attacks, our_attacks) {
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

    // 🎯 SEE - Static Exchange Evaluation (Swap List)
    
    /// Piece values indexed by piece type for SEE (same order as Piece enum).
    /// Index 0 = Empty, 1 = Pawn, 2 = Knight, 3 = Bishop, 4 = Rook, 5 = Queen, 6 = King.
    const SEE_VALUES: [i32; 7] = [0, 100, 320, 330, 500, 900, 20000];

    /// Returns a bitboard of ALL pieces (both colors) that attack `square`
    /// given the supplied `occupied` mask. Recomputing with an updated occupancy
    /// is what reveals x-ray / battery attacks after a piece is removed.
    #[inline(always)]
    pub fn attackers_to(board: &BoardState, square: u8, occupied: u64, tables: &AttackTables) -> u64 {
        // SEE changes occupancy but keeps the board bitboards immutable.
        // Mask pawns too, otherwise exchanged pawns remain ghost attackers.
        let all_pawns_w = board.pieces[0][Piece::Pawn as usize] & occupied;
        let all_pawns_b = board.pieces[1][Piece::Pawn as usize] & occupied;
        let knights = (board.pieces[0][Piece::Knight as usize]
                     | board.pieces[1][Piece::Knight as usize]) & occupied;
        let kings   = (board.pieces[0][Piece::King as usize]
                     | board.pieces[1][Piece::King as usize]) & occupied;
        let diag_sliders = ((board.pieces[0][Piece::Bishop as usize]
                           | board.pieces[1][Piece::Bishop as usize])
                          | (board.pieces[0][Piece::Queen as usize]
                           | board.pieces[1][Piece::Queen as usize])) & occupied;
        let orth_sliders = ((board.pieces[0][Piece::Rook as usize]
                           | board.pieces[1][Piece::Rook as usize])
                          | (board.pieces[0][Piece::Queen as usize]
                           | board.pieces[1][Piece::Queen as usize])) & occupied;

        // Reverse‐lookup: "which pawns attack this square?"
        // White pawns that attack `square` are found via black's pawn-attack pattern.
        let pawn_att = (tables.pawn_attacks[1][square as usize] & all_pawns_w)
                     | (tables.pawn_attacks[0][square as usize] & all_pawns_b);

        pawn_att
            | (tables.knight_attacks[square as usize] & knights)
            | (tables.get_bishop_attacks(square, occupied) & diag_sliders)
            | (tables.get_rook_attacks(square, occupied)   & orth_sliders)
            | (tables.king_attacks[square as usize]   & kings)
    }

    /// Among the bits set in `attackers`, find the square of the least valuable
    /// piece belonging to `color`. Returns `(square, piece_type_index)` or None.
    #[inline(always)]
    fn find_least_valuable_attacker_from(
        attackers: u64,
        board: &BoardState,
        color: usize,
    ) -> Option<(u8, usize)> {
        // Iterate in LVA order: Pawn(1) → Knight(2) → Bishop(3) → Rook(4) → Queen(5) → King(6)
        for pt in 1..=6usize {
            let subset = attackers & board.pieces[color][pt];
            if subset != 0 {
                let sq = subset.trailing_zeros() as u8;
                return Some((sq, pt));
            }
        }
        None
    }

    /// Full swap‐list SEE for a capture move from `from` → `to`.
    ///
    /// `target_piece` is the piece type index (1–6) of the piece currently on `to`
    /// (the initial victim). `side` is the color (0/1) of the side making the
    /// initial capture (i.e., the side that owns the piece on `from`).
    ///
    /// Returns the net material gain from the perspective of `side`.
    pub fn see(board: &BoardState, from: u8, to: u8, target_piece: usize, side: usize) -> i32 {
        Self::see_internal(board, from, to, target_piece, side, None, None)
    }

    /// SEE for a concrete move, including special capture and promotion semantics.
    fn see_internal(
        board: &BoardState,
        from: u8,
        to: u8,
        target_piece: usize,
        side: usize,
        promotion: Option<Piece>,
        captured_off_square: Option<u8>,
    ) -> i32 {
        let tables = &ATTACK_TABLES;
        let mut gain: [i32; 32] = [0; 32];
        let mut depth: usize = 0;

        let attacker_piece = {
            let mut found = 0usize;
            for pt in 1..=6usize {
                if board.pieces[side][pt] & (1u64 << from) != 0 {
                    found = pt;
                    break;
                }
            }
            found
        };
        if attacker_piece == 0 {
            return 0;
        }

        let promotion_gain = promotion
            .map(|piece| Self::SEE_VALUES[piece as usize] - Self::SEE_VALUES[Piece::Pawn as usize])
            .unwrap_or(0);
        gain[0] = Self::SEE_VALUES[target_piece] + promotion_gain;

        // Remove the mover and, for en passant, the captured pawn behind the target.
        let mut occupied = board.all_pieces & !(1u64 << from);
        if let Some(captured_sq) = captured_off_square {
            occupied &= !(1u64 << captured_sq);
        }

        let mut current_piece_value = promotion
            .map(|piece| Self::SEE_VALUES[piece as usize])
            .unwrap_or(Self::SEE_VALUES[attacker_piece]);

        let mut side_to_move = 1 - side;
        let mut all_attackers = Self::attackers_to(board, to, occupied, tables) & occupied;

        loop {
            depth += 1;
            if depth >= 32 {
                break;
            }

            gain[depth] = current_piece_value - gain[depth - 1];
            if (-gain[depth - 1]).max(gain[depth]) < 0 {
                break;
            }

            let (attacker_sq, piece_type) =
                match Self::find_least_valuable_attacker_from(all_attackers, board, side_to_move) {
                    Some(value) => value,
                    None => break,
                };

            if piece_type == Piece::King as usize {
                let opponent = 1 - side_to_move;
                if all_attackers & board.color_bb[opponent] & occupied != 0 {
                    break;
                }
            }

            occupied ^= 1u64 << attacker_sq;
            current_piece_value = Self::SEE_VALUES[piece_type];
            all_attackers = Self::attackers_to(board, to, occupied, tables) & occupied;
            side_to_move = 1 - side_to_move;
        }

        while depth > 1 {
            depth -= 1;
            gain[depth - 1] = -((-gain[depth - 1]).max(gain[depth]));
        }

        gain[0]
    }

    /// Evaluate whether a move `mv` has SEE ≥ `threshold`.
    /// Now handles both captures and quiet moves.
    pub fn see_capture(board: &BoardState, mv: &crate::movegen::Move, threshold: i32) -> bool {
        let side = board.side_to_move as usize;
        let (target_piece, captured_off_square) =
            if mv.flags == crate::movegen::EP_CAPTURE {
                let captured_sq = if board.side_to_move == Color::White {
                    mv.to - 8
                } else {
                    mv.to + 8
                };
                (Piece::Pawn as usize, Some(captured_sq))
            } else {
                (
                    board.piece_at(mv.to)
                        .map(|(piece, _)| piece as usize)
                        .unwrap_or(0),
                    None,
                )
            };

        Self::see_internal(
            board,
            mv.from,
            mv.to,
            target_piece,
            side,
            mv.promotion_piece(),
            captured_off_square,
        ) >= threshold
    }

    /// Evaluate whether a piece on `square` (owned by `defender_color`) is safe.
    fn see_square(board: &BoardState, square: u8, defender_color: u8) -> i32 {
        let attacker_color = (1 - defender_color) as usize;
        let tables = &ATTACK_TABLES;

        let (piece, _) = match board.piece_at(square) {
            Some(p) => p,
            None => return 0,
        };

        let all_attackers = Self::attackers_to(board, square, board.all_pieces, tables);
        let enemy_attackers = all_attackers & board.color_bb[attacker_color];
        if enemy_attackers == 0 {
            return 0;
        }

        let (attacker_sq, _) = match Self::find_least_valuable_attacker_from(
            enemy_attackers, board, attacker_color,
        ) {
            Some(v) => v,
            None => return 0,
        };

        let see_value = Self::see(board, attacker_sq, square, piece as usize, attacker_color);

        if see_value > 0 {
            -see_value
        } else {
            0
        }
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
    fn is_piece_trapped(
        board: &BoardState, 
        square: u8, 
        piece_type: usize, 
        color: usize, 
        tables: &AttackTables,
        enemy_attacks: Bitboard,
        our_attacks: Bitboard
    ) -> bool {
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
        
        let mut temp_moves = moves & !board.color_bb[color];
        
        // Count safe squares
        let mut safe_count = 0;
        
        while temp_moves != 0 {
            let (new_bb, sq) = pop_lsb(temp_moves);
            temp_moves = new_bb;
            let target_bb = 1u64 << sq.unwrap();
            
            // A square is considered "safe" for escaping or trading if:
            // 1. It is not attacked by the enemy.
            // 2. It IS attacked but also defended by us (offering a trade).
            // HIGH-VALUE PIECE SAFETY: Only count defended squares as safe for minor pieces (Knight/Bishop).
            // For Queens and Rooks, we remain pessimistic and only count unattacked squares.
            let is_high_value = piece_type == 4 || piece_type == 5; // Rook or Queen
            
            if (enemy_attacks & target_bb) == 0 || (!is_high_value && (our_attacks & target_bb) != 0) {
                safe_count += 1;
            }
        }
        
        // If fewer than 2 safe/trade squares, piece is likely trapped
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
    
    fn pawn_chains(board: &BoardState, _phase: i32) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];
        let tables = &ATTACK_TABLES;

        // White pawn chains — count pawns defended by other pawns
        let mut temp = white_pawns;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            
            // Check if defended by a pawn (diagonal behind)
            let pawn_defenders = tables.pawn_attacks[1][square as usize] & white_pawns;
            if pawn_defenders != 0 {
                score += PAWN_CHAIN_BONUS;
            }
        }
        
        // Black pawn chains
        let mut temp = black_pawns;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let square = sq.unwrap();
            
            let pawn_defenders = tables.pawn_attacks[0][square as usize] & black_pawns;
            if pawn_defenders != 0 {
                score -= PAWN_CHAIN_BONUS;
            }
        }
        
        // Backward pawns — pawns that can't advance because enemy pawns control the stop square
        // and no friendly pawns on adjacent files can support them
        for file in 0..8u8 {
            let file_mask = FILE_A << file;
            
            // White backward pawns
            let mut pawns_on_file = white_pawns & file_mask;
            while pawns_on_file != 0 {
                let (new_bb, sq) = pop_lsb(pawns_on_file);
                pawns_on_file = new_bb;
                let square = sq.unwrap();
                let rank = square / 8;
                
                if rank >= 6 { continue; } // 7th rank can't be backward
                
                let stop_square = square + 8;
                
                // Check if enemy pawns attack the stop square
                let stop_attacked = tables.pawn_attacks[1][stop_square as usize] & black_pawns;
                if stop_attacked == 0 { continue; }
                
                // Check if any friendly pawn on adjacent files can support
                let mut has_support = false;
                for adj in [file.wrapping_sub(1), file + 1] {
                    if adj < 8 {
                        let adj_mask = FILE_A << adj;
                        let friendly_adj = white_pawns & adj_mask;
                        // Check if any pawn on adjacent file is on same rank or behind
                        let mut check = friendly_adj;
                        while check != 0 {
                            let (nb, ps) = pop_lsb(check);
                            check = nb;
                            let p_rank = ps.unwrap() / 8;
                            if p_rank <= rank {
                                has_support = true;
                                break;
                            }
                        }
                    }
                    if has_support { break; }
                }
                
                if !has_support {
                    score -= BACKWARD_PAWN;
                }
            }
            
            // Black backward pawns
            let mut pawns_on_file = black_pawns & file_mask;
            while pawns_on_file != 0 {
                let (new_bb, sq) = pop_lsb(pawns_on_file);
                pawns_on_file = new_bb;
                let square = sq.unwrap();
                let rank = square / 8;
                
                if rank <= 1 { continue; }
                
                let stop_square = square - 8;
                
                let stop_attacked = tables.pawn_attacks[0][stop_square as usize] & white_pawns;
                if stop_attacked == 0 { continue; }
                
                let mut has_support = false;
                for adj in [file.wrapping_sub(1), file + 1] {
                    if adj < 8 {
                        let adj_mask = FILE_A << adj;
                        let friendly_adj = black_pawns & adj_mask;
                        let mut check = friendly_adj;
                        while check != 0 {
                            let (nb, ps) = pop_lsb(check);
                            check = nb;
                            let p_rank = ps.unwrap() / 8;
                            if p_rank >= rank {
                                has_support = true;
                                break;
                            }
                        }
                    }
                    if has_support { break; }
                }
                
                if !has_support {
                    score += BACKWARD_PAWN;
                }
            }
        }
        
        score
    }

    fn count_king_zone_attackers(board: &BoardState, king_sq: u8, by_color: Color) -> i32 {
        let tables = &ATTACK_TABLES;
        let king_zone = tables.king_attacks[king_sq as usize] | (1u64 << king_sq);
        let color = by_color as usize;
        
        let mut attackers = 0;
        
        // Weight attackers by piece type — queens are far more dangerous than knights
        let piece_weights: [i32; 6] = [0, 0, 2, 2, 3, 5]; // Pawn, Knight, Bishop, Rook, Queen
        
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
                    attackers += piece_weights[piece_type];
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
            // Include tradeable squares (attacked and defended)
            let safe_attacks = attacks & (!black_attacks | white_attacks);
            white_mobility += count_bits(safe_attacks) as i32 * KNIGHT_MOBILITY;
        }

        let mut knights = board.pieces[1][Piece::Knight as usize];
        while knights != 0 {
            let (new_bb, sq) = pop_lsb(knights);
            knights = new_bb;
            let square = sq.unwrap();
            let attacks = tables.knight_attacks[square as usize] & !board.color_bb[1];
            let safe_attacks = attacks & (!white_attacks | black_attacks);
            black_mobility += count_bits(safe_attacks) as i32 * KNIGHT_MOBILITY;
        }

        // Bishops - safe mobility
        let mut bishops = board.pieces[0][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[0];
            let safe_attacks = attacks & (!black_attacks | white_attacks);
            white_mobility += count_bits(safe_attacks) as i32 * BISHOP_MOBILITY;
        }

        let mut bishops = board.pieces[1][Piece::Bishop as usize];
        while bishops != 0 {
            let (new_bb, sq) = pop_lsb(bishops);
            bishops = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_bishop_attacks(square, board.all_pieces) & !board.color_bb[1];
            let safe_attacks = attacks & (!white_attacks | black_attacks);
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
            // High-value pieces (Queens/Rooks) only count unattacked squares for safe mobility
            let safe_attacks = attacks & !black_attacks;
            white_mobility += count_bits(safe_attacks) as i32 * QUEEN_MOBILITY;
        }

        let mut queens = board.pieces[1][Piece::Queen as usize];
        while queens != 0 {
            let (new_bb, sq) = pop_lsb(queens);
            queens = new_bb;
            let square = sq.unwrap();
            let attacks = tables.get_queen_attacks(square, board.all_pieces) & !board.color_bb[1];
            // High-value pieces (Queens/Rooks) only count unattacked squares for safe mobility
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
                let attack_index = (attackers as usize).min(7);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            // Penalty for king in center during middlegame
            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank <= 2 {
                safety -= 50;
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
                let attack_index = (attackers as usize).min(7);
                safety -= KING_ATTACK_WEIGHT[attack_index];
            }

            if phase > 18 && king_file >= 2 && king_file <= 5 && king_rank >= 5 {
                safety -= 50;
            }

            if board.castling_rights & 12 != 0 {
                safety += CASTLING_RIGHTS_BONUS;
            }

            score -= (safety * phase) / 24;
        }

        score
    }

    // ══════════════════════════════════════════════════════════════════════════════
    // PIECE CONNECTIVITY (pieces protecting each other)
    // ══════════════════════════════════════════════════════════════════════════════
    
    fn piece_connectivity(board: &BoardState, phase: i32) -> i32 {
        let tables = &ATTACK_TABLES;
        let white_attacks = build_attack_map(board, 0, tables);
        let black_attacks = build_attack_map(board, 1, tables);
        
        let mut score = 0;
        
        // White pieces defended by other white pieces
        for piece_type in 2..=5 { // Knight to Queen
            let pieces = board.pieces[0][piece_type];
            let defended = count_bits(pieces & white_attacks) as i32;
            score += defended * CONNECTIVITY_BONUS;
        }
        
        // Black pieces defended by other black pieces
        for piece_type in 2..=5 {
            let pieces = board.pieces[1][piece_type];
            let defended = count_bits(pieces & black_attacks) as i32;
            score -= defended * CONNECTIVITY_BONUS;
        }
        
        // Scale by phase (more valuable in middlegame)
        (score * phase) / 24
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