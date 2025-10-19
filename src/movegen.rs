use crate::board::{BoardState, Piece, Color};
use crate::bitboard::*;

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
    pub fn generate_legal_moves(board: &BoardState) -> Vec<Move> {
        let pseudo_legal = Self::generate_pseudo_legal(board);
        let mut legal_moves = Vec::with_capacity(pseudo_legal.len());

        for mv in pseudo_legal {
            let mut new_board = board.clone();
            new_board.make_move(&mv);
            
            // Check if own king is in check after move (illegal)
            if !new_board.is_in_check(board.side_to_move) {
                legal_moves.push(mv);
            }
        }

        legal_moves
    }

    pub fn generate_captures(board: &BoardState) -> Vec<Move> {
        Self::generate_legal_moves(board)
            .into_iter()
            .filter(|m| m.is_capture())
            .collect()
    }

    fn generate_pseudo_legal(board: &BoardState) -> Vec<Move> {
        let mut moves = Vec::with_capacity(256);
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

    fn generate_pawn_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_knight_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_bishop_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_rook_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_queen_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_king_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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

    fn generate_castling_moves(board: &BoardState, color: Color, moves: &mut Vec<Move>) {
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