use crate::board::BoardState;
use crate::search::SearchEngine;
use crate::movegen::{Move, MoveGenerator};
use std::io::{self, BufRead};

pub struct UCIEngine {
    board: BoardState,
    search_engine: SearchEngine,
    debug: bool,
}

impl UCIEngine {
    pub fn new() -> Self {
        UCIEngine {
            board: BoardState::default(),
            search_engine: SearchEngine::new(4),
            debug: false,
        }
    }

    pub fn run(&mut self) {
        let stdin = io::stdin();
        let reader = stdin.lock();

        for line in reader.lines() {
            if let Ok(command) = line {
                let command = command.trim();
                if !command.is_empty() {
                    if !self.handle_command(command) {
                        break;
                    }
                }
            }
        }
    }

    fn handle_command(&mut self, command: &str) -> bool {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return true;
        }

        match parts[0] {
            "uci" => self.uci(),
            "isready" => self.isready(),
            "ucinewgame" => self.ucinewgame(),
            "position" => self.position(&parts[1..]),
            "go" => self.go(&parts[1..]),
            "stop" => self.stop(),
            "quit" => return false,
            "debug" => {
                if parts.len() > 1 {
                    self.debug = parts[1] == "on";
                }
            }
            "setoption" => self.setoption(&parts[1..]),
            "d" => self.display(),
            _ => {
                if self.debug {
                    println!("info string Unknown command: {}", command);
                }
            }
        }

        true
    }

    fn uci(&self) {
        println!("id name RustChessEngine Ultimate v4.0");
        println!("id author StockfishKiller Team (Rust Edition)");
        println!("option name Hash type spin default 256 min 16 max 8192");
        println!("option name Threads type spin default 4 min 1 max 8");
        println!("option name ClearHash type button");
        println!("uciok");
    }

    fn isready(&self) {
        println!("readyok");
    }

    fn ucinewgame(&mut self) {
        self.search_engine.new_game();
        self.board = BoardState::default();
        if self.debug {
            println!("info string New game started");
        }
    }

    fn position(&mut self, args: &[&str]) {
        if args.is_empty() {
            return;
        }

        let mut move_idx = 1;

        if args[0] == "startpos" {
            self.board = BoardState::default();
        } else if args[0] == "fen" {
            let mut fen_parts = Vec::new();
            while move_idx < args.len() && args[move_idx] != "moves" {
                fen_parts.push(args[move_idx]);
                move_idx += 1;
            }
            
            let fen = fen_parts.join(" ");
            match BoardState::from_fen(&fen) {
                Ok(board) => self.board = board,
                Err(e) => {
                    if self.debug {
                        println!("info string Invalid FEN: {}", e);
                    }
                    return;
                }
            }
        } else {
            return;
        }

        // Apply moves
        if move_idx < args.len() && args[move_idx] == "moves" {
            for move_str in &args[move_idx + 1..] {
                if let Some(mv) = self.parse_uci_move(move_str) {
                    self.board.make_move(&mv);
                } else if self.debug {
                    println!("info string Invalid move: {}", move_str);
                }
            }
        }

        if self.board.is_repetition() && self.debug {
            println!("info string Position is a repetition");
        }
    }

    fn go(&mut self, args: &[&str]) {
        let mut depth = 64;
        let mut time_ms = None;
        let mut wtime = None;
        let mut btime = None;
        let mut winc = 0;
        let mut binc = 0;
        let mut movestogo = 40;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "depth" => {
                    if i + 1 < args.len() {
                        depth = args[i + 1].parse().unwrap_or(64);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "movetime" => {
                    if i + 1 < args.len() {
                        time_ms = Some(args[i + 1].parse().unwrap_or(1000));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "wtime" => {
                    if i + 1 < args.len() {
                        wtime = Some(args[i + 1].parse().unwrap_or(60000));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "btime" => {
                    if i + 1 < args.len() {
                        btime = Some(args[i + 1].parse().unwrap_or(60000));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "winc" => {
                    if i + 1 < args.len() {
                        winc = args[i + 1].parse().unwrap_or(0);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "binc" => {
                    if i + 1 < args.len() {
                        binc = args[i + 1].parse().unwrap_or(0);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "movestogo" => {
                    if i + 1 < args.len() {
                        movestogo = args[i + 1].parse().unwrap_or(40);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "infinite" => {
                    depth = 100;
                    time_ms = None;
                    i += 1;
                }
                _ => i += 1,
            }
        }

        // Smart time management
        if time_ms.is_none() {
            if let (Some(wt), Some(bt)) = (wtime, btime) {
                let my_time = if self.board.side_to_move == crate::board::Color::White {
                    wt
                } else {
                    bt
                };
                let my_inc = if self.board.side_to_move == crate::board::Color::White {
                    winc
                } else {
                    binc
                };

                // Adaptive time allocation
                let time_fraction = if movestogo > 0 {
                    1.0 / (movestogo + 5) as f64
                } else {
                    let moves_remaining = (50 - self.board.fullmove_number).max(30);
                    1.0 / moves_remaining as f64
                };

                let mut allocated = (my_time as f64 * time_fraction + my_inc as f64 * 0.7) as u64;

                // Game phase adjustments
                if self.board.fullmove_number < 10 {
                    allocated = (allocated as f64 * 0.75) as u64;
                } else if self.board.fullmove_number > 40 {
                    allocated = (allocated as f64 * 1.3) as u64;
                }

                // Check for critical positions
                if self.board.is_in_check(self.board.side_to_move) {
                    allocated = (allocated as f64 * 1.4) as u64;
                }

                // Safety margins
                let safety_margin = (my_time / 10).max(3000);
                allocated = allocated.min(my_time - safety_margin);

                // Absolute limits
                allocated = allocated.max(200).min(120000);

                time_ms = Some(allocated);

                if self.debug {
                    println!("info string Allocated {}ms for this move", allocated);
                }
            }
        }

        // Check for immediate draw
        if self.board.is_draw() && self.debug {
            println!("info string Position is drawn");
        }

        // Search
        let result = self.search_engine.search(
            self.board.clone(),
            depth,
            time_ms,
        );

        if let Some(best_move) = result.best_move {
            println!("bestmove {}", best_move.to_uci());
        } else {
            println!("bestmove 0000");
        }
    }

    fn stop(&mut self) {
        self.search_engine.stop();
    }

    fn setoption(&mut self, args: &[&str]) {
        if args.len() < 4 || args[0] != "name" {
            return;
        }

        let mut name_parts = Vec::new();
        let mut value_idx = 1;
        
        while value_idx < args.len() && args[value_idx] != "value" {
            name_parts.push(args[value_idx]);
            value_idx += 1;
        }

        let name = name_parts.join(" ").to_lowercase();

        // Button option (no value)
        if value_idx >= args.len() {
            if name == "clearhash" {
                self.search_engine.clear_tt();
                if self.debug {
                    println!("info string Hash table cleared");
                }
            }
            return;
        }

        if value_idx + 1 >= args.len() {
            return;
        }

        let value = args[value_idx + 1];

        match name.as_str() {
            "hash" => {
                if let Ok(size_mb) = value.parse::<usize>() {
                    self.search_engine.set_hash_size(size_mb);
                    if self.debug {
                        println!("info string Hash table set to {} MB", size_mb);
                    }
                }
            }
            "threads" => {
                if let Ok(threads) = value.parse::<usize>() {
                    self.search_engine.set_threads(threads);
                    if self.debug {
                        println!("info string Threads set to {}", threads);
                    }
                }
            }
            _ => {}
        }
    }

    fn display(&self) {
        println!("\n{}", self.board.to_fen());
        // Could print ASCII board here if desired
        println!();
    }

    fn parse_uci_move(&self, uci: &str) -> Option<Move> {
        if uci.len() < 4 {
            return None;
        }

        let from = parse_square(&uci[0..2])?;
        let to = parse_square(&uci[2..4])?;

        // Generate legal moves and find matching one
        let legal_moves = MoveGenerator::generate_legal_moves(&self.board);
        
        for mv in legal_moves {
            if mv.from == from && mv.to == to {
                // Check promotion if specified
                if uci.len() == 5 {
                    let promo_char = uci.chars().nth(4)?;
                    let promo_piece = mv.promotion_piece()?;
                    
                    let matches = match promo_char {
                        'n' => promo_piece == crate::board::Piece::Knight,
                        'b' => promo_piece == crate::board::Piece::Bishop,
                        'r' => promo_piece == crate::board::Piece::Rook,
                        'q' => promo_piece == crate::board::Piece::Queen,
                        _ => false,
                    };
                    
                    if matches {
                        return Some(mv);
                    }
                } else {
                    return Some(mv);
                }
            }
        }

        None
    }
}

fn parse_square(s: &str) -> Option<u8> {
    if s.len() != 2 {
        return None;
    }
    
    let file = s.chars().nth(0)? as u8;
    let rank = s.chars().nth(1)? as u8;
    
    if file < b'a' || file > b'h' || rank < b'1' || rank > b'8' {
        return None;
    }
    
    Some((rank - b'1') * 8 + (file - b'a'))
}

pub fn main() {
    let mut engine = UCIEngine::new();
    engine.run();
}