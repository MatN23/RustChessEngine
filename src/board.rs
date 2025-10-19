use crate::bitboard::*;
use crate::zobrist::ZOBRIST;
use crate::movegen::{Move, CAPTURE, EP_CAPTURE, DOUBLE_PAWN_PUSH, KING_CASTLE, QUEEN_CASTLE};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(usize)]
pub enum Piece {
    Empty = 0,
    Pawn = 1,
    Knight = 2,
    Bishop = 3,
    Rook = 4,
    Queen = 5,
    King = 6,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    pub fn flip(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

pub const PIECE_VALUES: [i32; 7] = [0, 100, 320, 330, 500, 900, 20000];

#[derive(Clone)]
pub struct BoardState {
    pub pieces: [[Bitboard; 7]; 2],
    pub color_bb: [Bitboard; 2],
    pub all_pieces: Bitboard,
    pub side_to_move: Color,
    pub castling_rights: u8,
    pub ep_square: Option<u8>,
    pub halfmove_clock: u16,
    pub fullmove_number: u16,
    pub hash: u64,
    pub position_history: VecDeque<u64>,
}

impl Default for BoardState {
    fn default() -> Self {
        Self::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
            .unwrap()
    }
}

impl BoardState {
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() < 4 {
            return Err("Invalid FEN".to_string());
        }

        let mut board = BoardState {
            pieces: [[0; 7]; 2],
            color_bb: [0; 2],
            all_pieces: 0,
            side_to_move: Color::White,
            castling_rights: 0,
            ep_square: None,
            halfmove_clock: 0,
            fullmove_number: 1,
            hash: 0,
            position_history: VecDeque::with_capacity(100),
        };

        // Parse piece placement
        let mut rank = 7i8;
        let mut file = 0i8;
        
        for ch in parts[0].chars() {
            if ch == '/' {
                rank -= 1;
                file = 0;
            } else if ch.is_numeric() {
                file += ch.to_digit(10).unwrap() as i8;
            } else {
                let sq = (rank * 8 + file) as u8;
                let color = if ch.is_uppercase() { Color::White } else { Color::Black };
                let piece = match ch.to_ascii_lowercase() {
                    'p' => Piece::Pawn,
                    'n' => Piece::Knight,
                    'b' => Piece::Bishop,
                    'r' => Piece::Rook,
                    'q' => Piece::Queen,
                    'k' => Piece::King,
                    _ => return Err(format!("Invalid piece: {}", ch)),
                };
                
                board.pieces[color as usize][piece as usize] = 
                    set_bit(board.pieces[color as usize][piece as usize], sq);
                board.color_bb[color as usize] = set_bit(board.color_bb[color as usize], sq);
                board.all_pieces = set_bit(board.all_pieces, sq);
                
                file += 1;
            }
        }

        board.side_to_move = if parts[1] == "w" { Color::White } else { Color::Black };

        if parts[2] != "-" {
            for ch in parts[2].chars() {
                board.castling_rights |= match ch {
                    'K' => 1,
                    'Q' => 2,
                    'k' => 4,
                    'q' => 8,
                    _ => 0,
                };
            }
        }

        if parts[3] != "-" {
            board.ep_square = Some(parse_square(parts[3])?);
        }

        if parts.len() > 4 {
            board.halfmove_clock = parts[4].parse().unwrap_or(0);
        }
        if parts.len() > 5 {
            board.fullmove_number = parts[5].parse().unwrap_or(1);
        }

        board.hash = board.compute_hash();
        board.position_history.push_back(board.hash);

        Ok(board)
    }

    pub fn to_fen(&self) -> String {
        let mut fen = String::new();
        
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                let sq = rank * 8 + file;
                if let Some((piece, color)) = self.piece_at(sq) {
                    if empty > 0 {
                        fen.push_str(&empty.to_string());
                        empty = 0;
                    }
                    let ch = match piece {
                        Piece::Pawn => 'p',
                        Piece::Knight => 'n',
                        Piece::Bishop => 'b',
                        Piece::Rook => 'r',
                        Piece::Queen => 'q',
                        Piece::King => 'k',
                        _ => continue,
                    };
                    fen.push(if color == Color::White { ch.to_ascii_uppercase() } else { ch });
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                fen.push_str(&empty.to_string());
            }
            if rank > 0 {
                fen.push('/');
            }
        }

        fen.push(' ');
        fen.push(if self.side_to_move == Color::White { 'w' } else { 'b' });

        fen.push(' ');
        if self.castling_rights == 0 {
            fen.push('-');
        } else {
            if self.castling_rights & 1 != 0 { fen.push('K'); }
            if self.castling_rights & 2 != 0 { fen.push('Q'); }
            if self.castling_rights & 4 != 0 { fen.push('k'); }
            if self.castling_rights & 8 != 0 { fen.push('q'); }
        }

        fen.push(' ');
        if let Some(sq) = self.ep_square {
            fen.push_str(&square_name(sq));
        } else {
            fen.push('-');
        }

        fen.push_str(&format!(" {} {}", self.halfmove_clock, self.fullmove_number));
        fen
    }

    pub fn piece_at(&self, sq: u8) -> Option<(Piece, Color)> {
        if !get_bit(self.all_pieces, sq) {
            return None;
        }

        let color = if get_bit(self.color_bb[0], sq) {
            Color::White
        } else {
            Color::Black
        };

        for piece_type in 1..=6 {
            if get_bit(self.pieces[color as usize][piece_type], sq) {
                return Some((
                    match piece_type {
                        1 => Piece::Pawn,
                        2 => Piece::Knight,
                        3 => Piece::Bishop,
                        4 => Piece::Rook,
                        5 => Piece::Queen,
                        6 => Piece::King,
                        _ => unreachable!(),
                    },
                    color,
                ));
            }
        }
        None
    }

    pub fn get_king_square(&self, color: Color) -> Option<u8> {
        lsb(self.pieces[color as usize][Piece::King as usize])
    }

    pub fn is_in_check(&self, color: Color) -> bool {
        if let Some(king_sq) = self.get_king_square(color) {
            self.is_square_attacked(king_sq, color.flip())
        } else {
            false
        }
    }

    pub fn is_square_attacked(&self, sq: u8, by_color: Color) -> bool {
        let tables = &ATTACK_TABLES;
        
        let pawn_attacks = tables.pawn_attacks[1 - by_color as usize][sq as usize];
        if pawn_attacks & self.pieces[by_color as usize][Piece::Pawn as usize] != 0 {
            return true;
        }

        let knight_attacks = tables.knight_attacks[sq as usize];
        if knight_attacks & self.pieces[by_color as usize][Piece::Knight as usize] != 0 {
            return true;
        }

        let king_attacks = tables.king_attacks[sq as usize];
        if king_attacks & self.pieces[by_color as usize][Piece::King as usize] != 0 {
            return true;
        }

        let bishop_attacks = tables.get_bishop_attacks(sq, self.all_pieces);
        if bishop_attacks & (self.pieces[by_color as usize][Piece::Bishop as usize] |
                            self.pieces[by_color as usize][Piece::Queen as usize]) != 0 {
            return true;
        }

        let rook_attacks = tables.get_rook_attacks(sq, self.all_pieces);
        if rook_attacks & (self.pieces[by_color as usize][Piece::Rook as usize] |
                          self.pieces[by_color as usize][Piece::Queen as usize]) != 0 {
            return true;
        }

        false
    }

    pub fn is_repetition(&self) -> bool {
        self.position_history.iter().filter(|&&h| h == self.hash).count() >= 2
    }

    pub fn is_draw(&self) -> bool {
        self.is_repetition() || 
        self.halfmove_clock >= 100 || 
        self.is_insufficient_material()
    }

    pub fn is_game_over(&self) -> bool {
        use crate::movegen::MoveGenerator;
        let moves = MoveGenerator::generate_legal_moves(self);
        moves.is_empty() || self.is_draw()
    }

    fn is_insufficient_material(&self) -> bool {
        let total_pieces = count_bits(self.all_pieces);
        
        if total_pieces == 2 {
            return true;
        }

        if total_pieces == 3 {
            let white_minor = count_bits(self.pieces[0][Piece::Knight as usize]) +
                             count_bits(self.pieces[0][Piece::Bishop as usize]);
            let black_minor = count_bits(self.pieces[1][Piece::Knight as usize]) +
                             count_bits(self.pieces[1][Piece::Bishop as usize]);
            
            if white_minor == 1 || black_minor == 1 {
                return true;
            }
        }

        false
    }

    fn compute_hash(&self) -> u64 {
        let mut hash = 0u64;
        
        for sq in 0..64 {
            if let Some((piece, color)) = self.piece_at(sq) {
                hash ^= ZOBRIST.piece_keys[color as usize][piece as usize][sq as usize];
            }
        }

        hash ^= ZOBRIST.castle_keys[self.castling_rights as usize];

        if let Some(ep_sq) = self.ep_square {
            hash ^= ZOBRIST.ep_keys[(ep_sq % 8) as usize];
        }

        if self.side_to_move == Color::Black {
            hash ^= ZOBRIST.side_key;
        }

        hash
    }

    pub fn make_move(&mut self, mv: &Move) {
        let from = mv.from;
        let to = mv.to;
        let flags = mv.flags;
        let color = self.side_to_move;
        
        if let Some((piece, _)) = self.piece_at(from) {
            // Update halfmove clock
            if piece == Piece::Pawn || mv.is_capture() {
                self.halfmove_clock = 0;
                self.position_history.clear();
            } else {
                self.halfmove_clock += 1;
            }

            // Clear old EP from hash
            if let Some(ep_sq) = self.ep_square {
                self.hash ^= ZOBRIST.ep_keys[(ep_sq % 8) as usize];
            }
            self.ep_square = None;

            // Handle captures
            if flags == CAPTURE || mv.is_promotion() && mv.is_capture() {
                if let Some((captured_piece, captured_color)) = self.piece_at(to) {
                    self.pieces[captured_color as usize][captured_piece as usize] = 
                        clear_bit(self.pieces[captured_color as usize][captured_piece as usize], to);
                    self.color_bb[captured_color as usize] = clear_bit(self.color_bb[captured_color as usize], to);
                    self.all_pieces = clear_bit(self.all_pieces, to);
                    self.hash ^= ZOBRIST.piece_keys[captured_color as usize][captured_piece as usize][to as usize];
                }
            } else if flags == EP_CAPTURE {
                let ep_captured_sq = if color == Color::White { to - 8 } else { to + 8 };
                let captured_color = color.flip();
                
                self.pieces[captured_color as usize][Piece::Pawn as usize] = 
                    clear_bit(self.pieces[captured_color as usize][Piece::Pawn as usize], ep_captured_sq);
                self.color_bb[captured_color as usize] = clear_bit(self.color_bb[captured_color as usize], ep_captured_sq);
                self.all_pieces = clear_bit(self.all_pieces, ep_captured_sq);
                self.hash ^= ZOBRIST.piece_keys[captured_color as usize][Piece::Pawn as usize][ep_captured_sq as usize];
            }

            // Move piece
            self.pieces[color as usize][piece as usize] = clear_bit(self.pieces[color as usize][piece as usize], from);
            self.color_bb[color as usize] = clear_bit(self.color_bb[color as usize], from);
            self.all_pieces = clear_bit(self.all_pieces, from);
            self.hash ^= ZOBRIST.piece_keys[color as usize][piece as usize][from as usize];

            // Handle promotions
            let final_piece = if let Some(promo_piece) = mv.promotion_piece() {
                promo_piece
            } else {
                piece
            };

            self.pieces[color as usize][final_piece as usize] = set_bit(self.pieces[color as usize][final_piece as usize], to);
            self.color_bb[color as usize] = set_bit(self.color_bb[color as usize], to);
            self.all_pieces = set_bit(self.all_pieces, to);
            self.hash ^= ZOBRIST.piece_keys[color as usize][final_piece as usize][to as usize];

            // Castling
            if flags == KING_CASTLE {
                let (rook_from, rook_to) = if color == Color::White { (7, 5) } else { (63, 61) };
                
                self.pieces[color as usize][Piece::Rook as usize] = clear_bit(self.pieces[color as usize][Piece::Rook as usize], rook_from);
                self.pieces[color as usize][Piece::Rook as usize] = set_bit(self.pieces[color as usize][Piece::Rook as usize], rook_to);
                self.color_bb[color as usize] = clear_bit(self.color_bb[color as usize], rook_from);
                self.color_bb[color as usize] = set_bit(self.color_bb[color as usize], rook_to);
                self.all_pieces = clear_bit(self.all_pieces, rook_from);
                self.all_pieces = set_bit(self.all_pieces, rook_to);
                
                self.hash ^= ZOBRIST.piece_keys[color as usize][Piece::Rook as usize][rook_from as usize];
                self.hash ^= ZOBRIST.piece_keys[color as usize][Piece::Rook as usize][rook_to as usize];
            } else if flags == QUEEN_CASTLE {
                let (rook_from, rook_to) = if color == Color::White { (0, 3) } else { (56, 59) };
                
                self.pieces[color as usize][Piece::Rook as usize] = clear_bit(self.pieces[color as usize][Piece::Rook as usize], rook_from);
                self.pieces[color as usize][Piece::Rook as usize] = set_bit(self.pieces[color as usize][Piece::Rook as usize], rook_to);
                self.color_bb[color as usize] = clear_bit(self.color_bb[color as usize], rook_from);
                self.color_bb[color as usize] = set_bit(self.color_bb[color as usize], rook_to);
                self.all_pieces = clear_bit(self.all_pieces, rook_from);
                self.all_pieces = set_bit(self.all_pieces, rook_to);
                
                self.hash ^= ZOBRIST.piece_keys[color as usize][Piece::Rook as usize][rook_from as usize];
                self.hash ^= ZOBRIST.piece_keys[color as usize][Piece::Rook as usize][rook_to as usize];
            }

            // Double pawn push
            if flags == DOUBLE_PAWN_PUSH {
                self.ep_square = Some(if color == Color::White { to - 8 } else { to + 8 });
                if let Some(ep_sq) = self.ep_square {
                    self.hash ^= ZOBRIST.ep_keys[(ep_sq % 8) as usize];
                }
            }

            // Update castling rights
            let old_castling = self.castling_rights;
            
            if piece == Piece::King {
                if color == Color::White {
                    self.castling_rights &= !(1 | 2);
                } else {
                    self.castling_rights &= !(4 | 8);
                }
            }

            if piece == Piece::Rook || mv.is_capture() {
                if from == 0 || to == 0 { self.castling_rights &= !2; }
                if from == 7 || to == 7 { self.castling_rights &= !1; }
                if from == 56 || to == 56 { self.castling_rights &= !8; }
                if from == 63 || to == 63 { self.castling_rights &= !4; }
            }

            if old_castling != self.castling_rights {
                self.hash ^= ZOBRIST.castle_keys[old_castling as usize];
                self.hash ^= ZOBRIST.castle_keys[self.castling_rights as usize];
            }
        }

        // Switch side
        self.side_to_move = self.side_to_move.flip();
        self.hash ^= ZOBRIST.side_key;

        // Update fullmove
        if self.side_to_move == Color::White {
            self.fullmove_number += 1;
        }

        // Add to position history
        self.position_history.push_back(self.hash);
    }

    pub fn make_move_uci(&mut self, uci: &str) -> Result<bool, String> {
        use crate::movegen::MoveGenerator;
        
        let legal_moves = MoveGenerator::generate_legal_moves(self);
        
        if uci.len() < 4 {
            return Err("Invalid UCI move".to_string());
        }

        let from = parse_square(&uci[0..2])?;
        let to = parse_square(&uci[2..4])?;

        for mv in legal_moves {
            if mv.from == from && mv.to == to {
                if uci.len() == 5 {
                    let promo_char = uci.chars().nth(4).unwrap();
                    if let Some(promo_piece) = mv.promotion_piece() {
                        let matches = match promo_char {
                            'n' => promo_piece == Piece::Knight,
                            'b' => promo_piece == Piece::Bishop,
                            'r' => promo_piece == Piece::Rook,
                            'q' => promo_piece == Piece::Queen,
                            _ => false,
                        };
                        
                        if matches {
                            self.make_move(&mv);
                            return Ok(true);
                        }
                    }
                } else {
                    self.make_move(&mv);
                    return Ok(true);
                }
            }
        }

        Err("Illegal move".to_string())
    }
}

pub fn parse_square(s: &str) -> Result<u8, String> {
    if s.len() != 2 {
        return Err("Invalid square".to_string());
    }
    let file = s.chars().nth(0).unwrap() as u8 - b'a';
    let rank = s.chars().nth(1).unwrap() as u8 - b'1';
    if file > 7 || rank > 7 {
        return Err("Invalid square".to_string());
    }
    Ok(rank * 8 + file)
}

pub fn square_name(sq: u8) -> String {
    let file = (b'a' + (sq % 8)) as char;
    let rank = (b'1' + (sq / 8)) as char;
    format!("{}{}", file, rank)
}