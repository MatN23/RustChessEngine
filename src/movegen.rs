use crate::board::{BoardState, Piece, Color};
use crate::bitboard::*;
use smallvec::SmallVec;

pub type MoveList = SmallVec<[Move; 256]>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    pub flags: u8,
}

// Move flags
pub const QUIET_MOVE: u8 = 0;
pub const DOUBLE_PAWN_PUSH: u8 = 1;
pub const KING_CASTLE: u8 = 2;
pub const QUEEN_CASTLE: u8 = 3;
pub const CAPTURE: u8 = 4;
pub const EP_CAPTURE: u8 = 5;
pub const KNIGHT_PROMOTION: u8 = 8;
pub const BISHOP_PROMOTION: u8 = 9;
pub const ROOK_PROMOTION: u8 = 10;
pub const QUEEN_PROMOTION: u8 = 11;
pub const KNIGHT_PROMO_CAPTURE: u8 = 12;
pub const BISHOP_PROMO_CAPTURE: u8 = 13;
pub const ROOK_PROMO_CAPTURE: u8 = 14;
pub const QUEEN_PROMO_CAPTURE: u8 = 15;

impl Move {
    pub fn new(from: u8, to: u8, flags: u8) -> Self {
        Move { from, to, flags }
    }

    pub fn to_uci(&self) -> String {
        let from_str = square_name(self.from);
        let to_str = square_name(self.to);
        
        if self.flags >= KNIGHT_PROMOTION {
            let promo = match self.flags {
                KNIGHT_PROMOTION | KNIGHT_PROMO_CAPTURE => "n",
                BISHOP_PROMOTION | BISHOP_PROMO_CAPTURE => "b",
                ROOK_PROMOTION | ROOK_PROMO_CAPTURE => "r",
                _ => "q",
            };
            format!("{}{}{}", from_str, to_str, promo)
        } else {
            format!("{}{}", from_str, to_str)
        }
    }

    pub fn is_capture(&self) -> bool {
        self.flags == CAPTURE || self.flags == EP_CAPTURE || self.flags >= KNIGHT_PROMO_CAPTURE
    }

    pub fn is_promotion(&self) -> bool {
        self.flags >= KNIGHT_PROMOTION
    }

    pub fn promotion_piece(&self) -> Option<Piece> {
        match self.flags {
            KNIGHT_PROMOTION | KNIGHT_PROMO_CAPTURE => Some(Piece::Knight),
            BISHOP_PROMOTION | BISHOP_PROMO_CAPTURE => Some(Piece::Bishop),
            ROOK_PROMOTION | ROOK_PROMO_CAPTURE => Some(Piece::Rook),
            QUEEN_PROMOTION | QUEEN_PROMO_CAPTURE => Some(Piece::Queen),
            _ => None,
        }
    }
}

pub struct MoveGenerator;

impl MoveGenerator {
    pub fn generate_legal_moves(board: &BoardState) -> MoveList {
        let pseudo_legal = Self::generate_pseudo_legal(board);
        Self::filter_legal(board, pseudo_legal)
    }

    /// Legal captures and promotions only (quiet promotions included).
    /// Legality is tested only for the few moves that survive the filter, so
    /// quiescence no longer pays for a full legal move generation per node.
    pub fn generate_captures(board: &BoardState) -> MoveList {
        let mut tactical = Self::generate_pseudo_legal(board);
        tactical.retain(|m| m.is_capture() || m.is_promotion());
        Self::filter_legal(board, tactical)
    }

    /// Removes pseudo-legal moves that leave the mover's king attacked.
    /// A move can only expose the king if the king is already in check, the
    /// king itself moves, it is en passant (two pawns leave a rank), or the
    /// moving piece stands on a line through the king (possible pin). Every
    /// other move is legal without making it.
    fn filter_legal(board: &BoardState, pseudo_legal: MoveList) -> MoveList {
        let us = board.side_to_move;
        let king_sq = match board.get_king_square(us) {
            Some(sq) => sq,
            None => return pseudo_legal,
        };
        let in_check = board.is_square_attacked(king_sq, us.flip());
        let king_lines = ATTACK_TABLES.get_queen_attacks(king_sq, 0);
        let mut legal_moves = MoveList::with_capacity(pseudo_legal.len());

        for mv in pseudo_legal {
            let needs_test = in_check
                || mv.from == king_sq
                || mv.flags == EP_CAPTURE
                || (king_lines >> mv.from) & 1 != 0;
            if needs_test {
                let mut new_board = board.clone_lite();
                new_board.make_move(&mv);
                if new_board.is_in_check(us) {
                    continue;
                }
            }
            legal_moves.push(mv);
        }

        legal_moves
    }

    fn generate_pseudo_legal(board: &BoardState) -> MoveList {
        let mut moves = MoveList::with_capacity(256);
        let color = board.side_to_move;

        Self::generate_pawn_moves(board, color, &mut moves);
        Self::generate_knight_moves(board, color, &mut moves);
        Self::generate_bishop_moves(board, color, &mut moves);
        Self::generate_rook_moves(board, color, &mut moves);
        Self::generate_queen_moves(board, color, &mut moves);
        Self::generate_king_moves(board, color, &mut moves);
        Self::generate_castling_moves(board, color, &mut moves);

        moves
    }

    fn generate_pawn_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let pawns = board.pieces[color as usize][Piece::Pawn as usize];
        let direction: i8 = if color == Color::White { 8 } else { -8 };
        let start_rank = if color == Color::White { 1 } else { 6 };
        let promo_rank = if color == Color::White { 7 } else { 0 };
        
        let enemy = board.color_bb[color.flip() as usize];
        let empty = !board.all_pieces;

        let mut temp_pawns = pawns;
        while temp_pawns != 0 {
            let (new_bb, sq) = pop_lsb(temp_pawns);
            temp_pawns = new_bb;
            let from_sq = sq.unwrap();
            let rank = from_sq / 8;
            let file = from_sq % 8;

            // Single push
            let to_sq = (from_sq as i8 + direction) as u8;
            if to_sq < 64 && get_bit(empty, to_sq) {
                if to_sq / 8 == promo_rank {
                    // Promotions
                    moves.push(Move::new(from_sq, to_sq, QUEEN_PROMOTION));
                    moves.push(Move::new(from_sq, to_sq, ROOK_PROMOTION));
                    moves.push(Move::new(from_sq, to_sq, BISHOP_PROMOTION));
                    moves.push(Move::new(from_sq, to_sq, KNIGHT_PROMOTION));
                } else {
                    moves.push(Move::new(from_sq, to_sq, QUIET_MOVE));
                    
                    // Double push
                    if rank == start_rank {
                        let to_sq2 = (from_sq as i8 + 2 * direction) as u8;
                        if get_bit(empty, to_sq2) {
                            moves.push(Move::new(from_sq, to_sq2, DOUBLE_PAWN_PUSH));
                        }
                    }
                }
            }

            // Captures
            for capture_direction in [direction - 1, direction + 1] {
                let to_sq = (from_sq as i8 + capture_direction) as u8;
                
                if to_sq < 64 {
                    let to_file = to_sq % 8;
                    if (to_file as i8 - file as i8).abs() == 1 {
                        // Regular capture
                        if get_bit(enemy, to_sq) {
                            if to_sq / 8 == promo_rank {
                                moves.push(Move::new(from_sq, to_sq, QUEEN_PROMO_CAPTURE));
                                moves.push(Move::new(from_sq, to_sq, ROOK_PROMO_CAPTURE));
                                moves.push(Move::new(from_sq, to_sq, BISHOP_PROMO_CAPTURE));
                                moves.push(Move::new(from_sq, to_sq, KNIGHT_PROMO_CAPTURE));
                            } else {
                                moves.push(Move::new(from_sq, to_sq, CAPTURE));
                            }
                        }
                        
                        // En passant
                        if let Some(ep_sq) = board.ep_square {
                            if to_sq == ep_sq {
                                moves.push(Move::new(from_sq, to_sq, EP_CAPTURE));
                            }
                        }
                    }
                }
            }
        }
    }

    fn generate_knight_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let knights = board.pieces[color as usize][Piece::Knight as usize];
        let own_pieces = board.color_bb[color as usize];
        let tables = &ATTACK_TABLES;

        let mut temp = knights;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let from_sq = sq.unwrap();
            
            let mut attacks = tables.knight_attacks[from_sq as usize] & !own_pieces;
            
            while attacks != 0 {
                let (new_attacks, to) = pop_lsb(attacks);
                attacks = new_attacks;
                let to_sq = to.unwrap();
                
                let is_capture = get_bit(board.all_pieces, to_sq);
                let flag = if is_capture { CAPTURE } else { QUIET_MOVE };
                moves.push(Move::new(from_sq, to_sq, flag));
            }
        }
    }

    fn generate_bishop_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let bishops = board.pieces[color as usize][Piece::Bishop as usize];
        let own_pieces = board.color_bb[color as usize];
        let tables = &ATTACK_TABLES;

        let mut temp = bishops;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let from_sq = sq.unwrap();
            
            let mut attacks = tables.get_bishop_attacks(from_sq, board.all_pieces) & !own_pieces;
            
            while attacks != 0 {
                let (new_attacks, to) = pop_lsb(attacks);
                attacks = new_attacks;
                let to_sq = to.unwrap();
                
                let is_capture = get_bit(board.all_pieces, to_sq);
                let flag = if is_capture { CAPTURE } else { QUIET_MOVE };
                moves.push(Move::new(from_sq, to_sq, flag));
            }
        }
    }

    fn generate_rook_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let rooks = board.pieces[color as usize][Piece::Rook as usize];
        let own_pieces = board.color_bb[color as usize];
        let tables = &ATTACK_TABLES;

        let mut temp = rooks;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let from_sq = sq.unwrap();
            
            let mut attacks = tables.get_rook_attacks(from_sq, board.all_pieces) & !own_pieces;
            
            while attacks != 0 {
                let (new_attacks, to) = pop_lsb(attacks);
                attacks = new_attacks;
                let to_sq = to.unwrap();
                
                let is_capture = get_bit(board.all_pieces, to_sq);
                let flag = if is_capture { CAPTURE } else { QUIET_MOVE };
                moves.push(Move::new(from_sq, to_sq, flag));
            }
        }
    }

    fn generate_queen_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let queens = board.pieces[color as usize][Piece::Queen as usize];
        let own_pieces = board.color_bb[color as usize];
        let tables = &ATTACK_TABLES;

        let mut temp = queens;
        while temp != 0 {
            let (new_bb, sq) = pop_lsb(temp);
            temp = new_bb;
            let from_sq = sq.unwrap();
            
            let mut attacks = tables.get_queen_attacks(from_sq, board.all_pieces) & !own_pieces;
            
            while attacks != 0 {
                let (new_attacks, to) = pop_lsb(attacks);
                attacks = new_attacks;
                let to_sq = to.unwrap();
                
                let is_capture = get_bit(board.all_pieces, to_sq);
                let flag = if is_capture { CAPTURE } else { QUIET_MOVE };
                moves.push(Move::new(from_sq, to_sq, flag));
            }
        }
    }

    fn generate_king_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        let king = board.pieces[color as usize][Piece::King as usize];
        let own_pieces = board.color_bb[color as usize];
        let tables = &ATTACK_TABLES;

        if king == 0 {
            return;
        }

        let from_sq = lsb(king).unwrap();
        let mut attacks = tables.king_attacks[from_sq as usize] & !own_pieces;
        
        while attacks != 0 {
            let (new_attacks, to) = pop_lsb(attacks);
            attacks = new_attacks;
            let to_sq = to.unwrap();
            
            let is_capture = get_bit(board.all_pieces, to_sq);
            let flag = if is_capture { CAPTURE } else { QUIET_MOVE };
            moves.push(Move::new(from_sq, to_sq, flag));
        }
    }

    fn generate_castling_moves(board: &BoardState, color: Color, moves: &mut MoveList) {
        if color == Color::White {
            // Kingside castling
            if board.castling_rights & 1 != 0 {
                if !get_bit(board.all_pieces, 5) && !get_bit(board.all_pieces, 6) &&
                   !board.is_square_attacked(4, Color::Black) &&
                   !board.is_square_attacked(5, Color::Black) &&
                   !board.is_square_attacked(6, Color::Black) {
                    moves.push(Move::new(4, 6, KING_CASTLE));
                }
            }
            
            // Queenside castling
            if board.castling_rights & 2 != 0 {
                if !get_bit(board.all_pieces, 3) && !get_bit(board.all_pieces, 2) &&
                   !get_bit(board.all_pieces, 1) &&
                   !board.is_square_attacked(4, Color::Black) &&
                   !board.is_square_attacked(3, Color::Black) &&
                   !board.is_square_attacked(2, Color::Black) {
                    moves.push(Move::new(4, 2, QUEEN_CASTLE));
                }
            }
        } else {
            // Kingside castling
            if board.castling_rights & 4 != 0 {
                if !get_bit(board.all_pieces, 61) && !get_bit(board.all_pieces, 62) &&
                   !board.is_square_attacked(60, Color::White) &&
                   !board.is_square_attacked(61, Color::White) &&
                   !board.is_square_attacked(62, Color::White) {
                    moves.push(Move::new(60, 62, KING_CASTLE));
                }
            }
            
            // Queenside castling
            if board.castling_rights & 8 != 0 {
                if !get_bit(board.all_pieces, 59) && !get_bit(board.all_pieces, 58) &&
                   !get_bit(board.all_pieces, 57) &&
                   !board.is_square_attacked(60, Color::White) &&
                   !board.is_square_attacked(59, Color::White) &&
                   !board.is_square_attacked(58, Color::White) {
                    moves.push(Move::new(60, 58, QUEEN_CASTLE));
                }
            }
        }
    }
}

fn square_name(sq: u8) -> String {
    let file = (b'a' + (sq % 8)) as char;
    let rank = (b'1' + (sq / 8)) as char;
    format!("{}{}", file, rank)
}


#[cfg(test)]
mod tactical_move_tests {
    use super::*;

    #[test]
    fn tactical_generation_includes_quiet_promotions() {
        let board = BoardState::from_fen(
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1"
        ).unwrap();

        let tactical = MoveGenerator::generate_captures(&board);
        let promotions: Vec<_> = tactical.into_iter().filter(|mv| mv.is_promotion()).collect();

        assert_eq!(promotions.len(), 4);
        assert!(promotions.iter().all(|mv| !mv.is_capture()));
    }

    fn perft(board: &BoardState, depth: u32) -> u64 {
        let moves = MoveGenerator::generate_legal_moves(board);
        if depth == 1 {
            return moves.len() as u64;
        }
        moves
            .iter()
            .map(|mv| {
                let mut b = board.clone();
                b.make_move(mv);
                perft(&b, depth - 1)
            })
            .sum()
    }

    #[test]
    fn fast_legality_filter_matches_known_perft() {
        let cases = [
            ("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", 3, 8902),
            ("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", 3, 97862),
            ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", 4, 43238),
            ("8/8/8/8/k2Pp2Q/8/8/3K4 b - d3 0 1", 3, 863),
        ];
        for (fen, depth, expected) in cases {
            let board = BoardState::from_fen(fen).unwrap();
            assert_eq!(perft(&board, depth), expected, "perft mismatch for {}", fen);
        }
    }

    #[test]
    fn generate_captures_is_legal_subset_of_all_moves() {
        let board = BoardState::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let all = MoveGenerator::generate_legal_moves(&board);
        let caps = MoveGenerator::generate_captures(&board);
        let expected: Vec<_> = all.iter().filter(|m| m.is_capture() || m.is_promotion()).collect();
        assert_eq!(caps.len(), expected.len());
        assert!(caps.iter().all(|m| all.contains(m)));
    }
}
