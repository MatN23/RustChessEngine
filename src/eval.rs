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
const PASSED_PAWN_BONUS: [i32; 8] = [0, 8, 15, 30, 55, 90, 180, 0];
const PROTECTED_PASSED_PAWN: [i32; 8] = [0, 4, 8, 14, 22, 35, 50, 0];
const CANDIDATE_PASSED: [i32; 8] = [0, 8, 12, 22, 38, 60, 100, 0];
const PAWN_CHAIN_BONUS: i32 = 10;

// King Safety (much stronger)
const PAWN_SHIELD_BONUS: i32 = 20;
const OPEN_FILE_NEAR_KING: i32 = 35;
const KING_ZONE_ATTACK: i32 = 15;
const CASTLING_RIGHTS_BONUS: i32 = 35;
// Indexed by summed attacker weight (N/B=2, R=3, Q=5). Previously topped out at 1200cp, so one queen
// touching the king zone scored -700. Values here are UNTUNED placeholders; tune with Texel/SPSA.
const KING_ATTACK_WEIGHT: [i32; 8] = [0, 0, 2, 8, 18, 32, 50, 75];


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

pub struct Evaluator;

impl Evaluator {
    /// Per-term breakdown (White perspective) written to stderr; used by the `evalterms` UCI command.
    pub fn debug_terms(board: &BoardState) {
        let phase = Self::game_phase(board);
        let (mg, eg) = Self::material_and_pst(board);
        eprintln!("phase={} material+pst mg={} eg={} tapered={}", phase, mg, eg, Self::tapered_eval(mg, eg, phase));
        eprintln!("pawn_structure   {}", Self::pawn_structure(board, phase));
        eprintln!("passed_pawns     {}", Self::passed_pawn_evaluation(board, phase));
        eprintln!("pawn_chains      {}", Self::pawn_chains(board, phase));
        eprintln!("mobility         {}", Self::piece_mobility_safe(board, phase));
        eprintln!("king_safety      {}", Self::king_safety_advanced(board, phase));
        eprintln!("space            {}", Self::space_evaluation(board, phase));
        eprintln!("rook             {}", Self::rook_evaluation(board));
        eprintln!("bishop           {}", Self::bishop_evaluation(board));
        eprintln!("knight           {}", Self::knight_evaluation(board));
        eprintln!("tempo            {}", Self::tempo_bonus(board));
        eprintln!("connectivity     {}", Self::piece_connectivity(board, phase));
    }

    pub fn evaluate(board: &BoardState) -> i32 {
        // Quick draw detection
        if board.is_insufficient_material() || board.halfmove_clock >= 100 {
            return 0;
        }

        let phase = Self::game_phase(board);
        
        // Core evaluation components
        let (mg_score, eg_score) = Self::material_and_pst(board);
        let mut score = Self::tapered_eval(mg_score, eg_score, phase);

        
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

    // 🎯 SEE - Static Exchange Evaluation (Swap List)
    
    /// Piece values indexed by piece type for SEE (same order as Piece enum).
    /// Index 0 = Empty, 1 = Pawn, 2 = Knight, 3 = Bishop, 4 = Rook, 5 = Queen, 6 = King.
    const SEE_VALUES: [i32; 7] = [0, 100, 320, 330, 500, 900, 20000];

    /// Returns a bitboard of ALL pieces (both colors) that attack `square`
    /// given the supplied `occupied` mask. Recomputing with an updated occupancy
    /// is what reveals x-ray / battery attacks after a piece is removed.
    #[inline(always)]
    pub fn attackers_to(board: &BoardState, square: u8, occupied: u64, tables: &AttackTables) -> u64 {
        let all_pawns_w = board.pieces[0][Piece::Pawn as usize];
        let all_pawns_b = board.pieces[1][Piece::Pawn as usize];
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
        let tables = &ATTACK_TABLES;

        // Swap list on the stack (max 32 captures in a chain is more than enough).
        let mut gain: [i32; 32] = [0; 32];
        let mut depth: usize = 0;

        // The moving piece is the attacker piece that starts the sequence.
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

        // Initial gain: we capture target_piece.
        gain[0] = Self::SEE_VALUES[target_piece];

        // Simulated occupancy — remove the initial attacker.
        let mut occupied = board.all_pieces & !(1u64 << from);

        // The piece now sitting on `to` (i.e., the attacker that just moved).
        let mut current_piece_value = Self::SEE_VALUES[attacker_piece];

        // Side that gets to recapture next.
        let mut side_to_move = 1 - side;

        // Recompute attackers to `to` with updated occupancy (reveals x-rays).
        let mut all_attackers = Self::attackers_to(board, to, occupied, tables);
        // Only consider pieces still on the board.
        all_attackers &= occupied;

        loop {
            depth += 1;
            if depth >= 32 {
                break;
            }

            // Negamax-style: gain[d] = value_of_piece_just_placed - gain[d-1]
            gain[depth] = current_piece_value - gain[depth - 1];

            // Early prune: if even capturing for free can't improve the position
            // for either side, stop.
            if (-gain[depth - 1]).max(gain[depth]) < 0 {
                break;
            }

            // Find the least valuable attacker for `side_to_move`.
            let (attacker_sq, pt) = match Self::find_least_valuable_attacker_from(
                all_attackers, board, side_to_move,
            ) {
                Some(v) => v,
                None => break, // No more attackers for this side — sequence ends.
            };

            // Don't allow king captures if the opponent still has attackers.
            if pt == Piece::King as usize {
                let opponent = 1 - side_to_move;
                if all_attackers & board.color_bb[opponent] & occupied != 0 {
                    break;
                }
            }

            // Remove the attacker from occupancy (simulates moving it to `to`).
            occupied ^= 1u64 << attacker_sq;
            current_piece_value = Self::SEE_VALUES[pt];

            // Recompute attackers — this is what discovers x-ray sliders.
            all_attackers = Self::attackers_to(board, to, occupied, tables);
            all_attackers &= occupied;

            side_to_move = 1 - side_to_move;
        }

        // Backward propagation (minimax on the gain array).
        // At each depth, the side to move picks the better of:
        //   • stand pat  → keeps -gain[d-1]
        //   • recapture  → gets  gain[d]
        while depth > 1 {
            depth -= 1;
            gain[depth - 1] = -((-gain[depth - 1]).max(gain[depth]));
        }

        gain[0]
    }

    /// Evaluate whether a move `mv` has SEE ≥ `threshold`.
    /// Now handles both captures and quiet moves.
    pub fn see_capture(board: &BoardState, mv: &crate::movegen::Move, threshold: i32) -> bool {
        let (target_piece, side) = if mv.flags == crate::movegen::EP_CAPTURE {
            (Piece::Pawn as usize, if board.side_to_move == Color::White { 0 } else { 1 })
        } else {
            let target = match board.piece_at(mv.to) {
                Some((p, _)) => p as usize,
                None => 0, // Empty square for quiet moves
            };
            let side = if get_bit(board.color_bb[0], mv.from) { 0 } else { 1 };
            (target, side)
        };

        Self::see(board, mv.from, mv.to, target_piece, side) >= threshold
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
                    
                    // PST tables are written a8-first (PeSTO layout); squares here are a1=0, so White must be flipped.
                    let pst_sq = if color == 0 { square ^ 56 } else { square };
                    
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

        // Pawn-storm scoring is omitted until castling side and actual attack
        // potential are evaluated rather than inferred from file proximity.

        score
    }

    // 🏰 Pawn Storm Evaluation
    fn passed_pawn_evaluation(board: &BoardState, phase: i32) -> i32 {
        let mut score = 0;
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        let mut white = white_pawns;
        while white != 0 {
            let (remaining, square_opt) = pop_lsb(white);
            white = remaining;
            let square = square_opt.unwrap();
            let file = square % 8;
            let rank = square / 8;

            let mut enemy_pawns_ahead = 0u64;
            for r in (rank + 1)..8 {
                if file > 0 {
                    enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file - 1);
                }
                enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file);
                if file < 7 {
                    enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file + 1);
                }
            }

            if black_pawns & enemy_pawns_ahead == 0 {
                let mut bonus = PASSED_PAWN_BONUS[rank as usize];
                let mut protection_mask = 0u64;
                if file > 0 && square >= 9 {
                    protection_mask |= 1u64 << (square - 9);
                }
                if file < 7 && square >= 7 {
                    protection_mask |= 1u64 << (square - 7);
                }
                if white_pawns & protection_mask != 0 {
                    bonus += PROTECTED_PASSED_PAWN[rank as usize];
                }

                // A piece in the promotion file can blockade the pawn.
                let mut blockaded = false;
                for r in (rank + 1)..8 {
                    if get_bit(board.all_pieces, r * 8 + file) {
                        blockaded = true;
                        break;
                    }
                }
                if blockaded {
                    bonus /= 2;
                }

                // Use the queening square for king proximity. A distant king
                // must not generate a negative proximity bonus.
                let promotion_square = 56 + file;
                if let Some(king_sq) = board.get_king_square(Color::White) {
                    let distance = ((king_sq / 8) as i32 - (promotion_square / 8) as i32)
                        .abs()
                        .max(((king_sq % 8) as i32 - (promotion_square % 8) as i32).abs());
                    bonus += ((7 - distance).max(0) * (24 - phase)) / 12;
                }
                if let Some(enemy_king_sq) = board.get_king_square(Color::Black) {
                    let distance = ((enemy_king_sq / 8) as i32 - (promotion_square / 8) as i32)
                        .abs()
                        .max(((enemy_king_sq % 8) as i32 - (promotion_square % 8) as i32).abs());
                    bonus -= ((7 - distance).max(0) * (24 - phase)) / 8;
                }
                score += bonus;
            }
        }

        let mut black = black_pawns;
        while black != 0 {
            let (remaining, square_opt) = pop_lsb(black);
            black = remaining;
            let square = square_opt.unwrap();
            let file = square % 8;
            let rank = square / 8;

            let mut enemy_pawns_ahead = 0u64;
            for r in 0..rank {
                if file > 0 {
                    enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file - 1);
                }
                enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file);
                if file < 7 {
                    enemy_pawns_ahead = set_bit(enemy_pawns_ahead, r * 8 + file + 1);
                }
            }

            if white_pawns & enemy_pawns_ahead == 0 {
                let relative_rank = (7 - rank) as usize;
                let mut bonus = PASSED_PAWN_BONUS[relative_rank];
                let mut protection_mask = 0u64;
                if file > 0 && square + 7 < 64 {
                    protection_mask |= 1u64 << (square + 7);
                }
                if file < 7 && square + 9 < 64 {
                    protection_mask |= 1u64 << (square + 9);
                }
                if black_pawns & protection_mask != 0 {
                    bonus += PROTECTED_PASSED_PAWN[relative_rank];
                }

                let mut blockaded = false;
                for r in 0..rank {
                    if get_bit(board.all_pieces, r * 8 + file) {
                        blockaded = true;
                        break;
                    }
                }
                if blockaded {
                    bonus /= 2;
                }

                let promotion_square = file;
                if let Some(king_sq) = board.get_king_square(Color::Black) {
                    let distance = ((king_sq / 8) as i32 - (promotion_square / 8) as i32)
                        .abs()
                        .max(((king_sq % 8) as i32 - (promotion_square % 8) as i32).abs());
                    bonus += ((7 - distance).max(0) * (24 - phase)) / 12;
                }
                if let Some(enemy_king_sq) = board.get_king_square(Color::White) {
                    let distance = ((enemy_king_sq / 8) as i32 - (promotion_square / 8) as i32)
                        .abs()
                        .max(((enemy_king_sq % 8) as i32 - (promotion_square % 8) as i32).abs());
                    bonus -= ((7 - distance).max(0) * (24 - phase)) / 8;
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
        let white_bishops = board.pieces[0][Piece::Bishop as usize];
        let white_pawns = board.pieces[0][Piece::Pawn as usize];
        let black_bishops = board.pieces[1][Piece::Bishop as usize];
        let black_pawns = board.pieces[1][Piece::Pawn as usize];

        // White kingside: bishop g2, supported by f2, g3, or h2.
        if get_bit(white_bishops, 14)
            && (get_bit(white_pawns, 13)
                || get_bit(white_pawns, 22)
                || get_bit(white_pawns, 15))
        {
            score += FIANCHETTO_BONUS;
        }
        // White queenside: bishop b2, supported by a2, b3, or c2.
        if get_bit(white_bishops, 9)
            && (get_bit(white_pawns, 8)
                || get_bit(white_pawns, 17)
                || get_bit(white_pawns, 10))
        {
            score += FIANCHETTO_BONUS;
        }
        // Black kingside: bishop g7, supported by f7, g6, or h7.
        if get_bit(black_bishops, 54)
            && (get_bit(black_pawns, 53)
                || get_bit(black_pawns, 46)
                || get_bit(black_pawns, 55))
        {
            score -= FIANCHETTO_BONUS;
        }
        // Black queenside: bishop b7, supported by a7, b6, or c7.
        if get_bit(black_bishops, 49)
            && (get_bit(black_pawns, 48)
                || get_bit(black_pawns, 41)
                || get_bit(black_pawns, 50))
        {
            score -= FIANCHETTO_BONUS;
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

#[cfg(test)]
mod evaluation_regression_tests {
    use super::*;
    use crate::board::BoardState;

    #[test]
    fn dead_material_evaluates_to_zero() {
        let board =
            BoardState::from_fen("7k/8/8/8/8/8/4B3/4K3 w - - 0 1").unwrap();
        assert_eq!(Evaluator::evaluate(&board), 0);
    }

    #[test]
    fn fianchetto_bonus_uses_g2_not_g1() {
        let g2_bishop =
            BoardState::from_fen("7k/8/8/8/8/8/5PB1/7K w - - 0 1").unwrap();
        let g1_bishop =
            BoardState::from_fen("7k/8/8/8/8/8/5P2/6BK w - - 0 1").unwrap();
        assert_eq!(Evaluator::fianchetto_patterns(&g2_bishop), FIANCHETTO_BONUS);
        assert_eq!(Evaluator::fianchetto_patterns(&g1_bishop), 0);
    }

    #[test]
    fn black_fianchetto_bonus_uses_g7_not_g8() {
        let g7_bishop =
            BoardState::from_fen("7k/5pb1/8/8/8/8/8/7K w - - 0 1").unwrap();
        assert_eq!(Evaluator::fianchetto_patterns(&g7_bishop), -FIANCHETTO_BONUS);
    }
}
