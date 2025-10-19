use std::collections::HashMap;
use lazy_static::lazy_static;
use rand::Rng;

/// Opening book entry with multiple move options and weights
struct BookPosition {
    moves: Vec<(String, u32)>, // (move_uci, weight)
}

impl BookPosition {
    fn new() -> Self {
        BookPosition { moves: Vec::new() }
    }

    fn add_move(&mut self, move_uci: &str, weight: u32) {
        self.moves.push((move_uci.to_string(), weight));
    }

    fn get_random_move(&self) -> Option<String> {
        if self.moves.is_empty() {
            return None;
        }

        let total_weight: u32 = self.moves.iter().map(|(_, w)| w).sum();
        if total_weight == 0 {
            return None;
        }

        let mut rng = rand::thread_rng();
        let mut roll = rng.gen_range(0..total_weight);

        for (mv, weight) in &self.moves {
            if roll < *weight {
                return Some(mv.clone());
            }
            roll -= weight;
        }

        // Fallback to first move
        self.moves.first().map(|(mv, _)| mv.clone())
    }
}

lazy_static! {
    static ref OPENING_BOOK: HashMap<String, BookPosition> = build_opening_book();
}

/// Build the opening book with popular lines
fn build_opening_book() -> HashMap<String, BookPosition> {
    let mut book = HashMap::new();

    // Starting position
    add_position(&mut book, 
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        vec![
            ("e2e4", 40),  // King's Pawn
            ("d2d4", 35),  // Queen's Pawn
            ("c2c4", 15),  // English
            ("g1f3", 8),   // Reti
            ("g2g3", 2),   // King's Fianchetto
        ]
    );

    // === KING'S PAWN OPENINGS (1.e4) ===
    
    // After 1.e4
    add_position(&mut book,
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
        vec![
            ("e7e5", 45),  // King's Pawn
            ("c7c5", 30),  // Sicilian
            ("e7e6", 15),  // French
            ("c7c6", 8),   // Caro-Kann
            ("d7d5", 2),   // Scandinavian
        ]
    );

    // === KING'S PAWN GAME (1.e4 e5) ===
    
    // After 1.e4 e5
    add_position(&mut book,
        "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2",
        vec![
            ("g1f3", 60),  // King's Knight
            ("f2f4", 20),  // King's Gambit
            ("b1c3", 15),  // Vienna
            ("f1c4", 5),   // Bishop's Opening
        ]
    );

    // After 1.e4 e5 2.Nf3
    add_position(&mut book,
        "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",
        vec![
            ("b8c6", 70),  // Most common
            ("g8f6", 25),  // Petroff
            ("d7d6", 5),   // Philidor
        ]
    );

    // === RUY LOPEZ ===
    
    // After 1.e4 e5 2.Nf3 Nc6
    add_position(&mut book,
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
        vec![
            ("f1b5", 70),  // Ruy Lopez
            ("f1c4", 20),  // Italian
            ("d2d4", 8),   // Scotch
            ("b1c3", 2),   // Four Knights
        ]
    );

    // Ruy Lopez mainline
    add_position(&mut book,
        "r1bqkbnr/pppp1ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
        vec![
            ("a7a6", 60),  // Morphy Defense
            ("g8f6", 25),  // Berlin Defense
            ("f7f5", 10),  // Schliemann
            ("d7d6", 5),   // Steinitz Defense
        ]
    );

    // Ruy Lopez, Morphy Defense
    add_position(&mut book,
        "r1bqkbnr/1ppp1ppp/p1n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4",
        vec![
            ("b5a4", 90),  // Main line
            ("b5c6", 10),  // Exchange variation
        ]
    );

    // After 4.Ba4
    add_position(&mut book,
        "r1bqkbnr/1ppp1ppp/p1n5/4p3/B3P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 1 4",
        vec![
            ("g8f6", 80),  // Main line
            ("f7f5", 15),  // Schliemann delayed
            ("d7d6", 5),
        ]
    );

    // After 4.Ba4 Nf6
    add_position(&mut book,
        "r1bqkb1r/1ppp1ppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 2 5",
        vec![
            ("e1g1", 85),  // Castle
            ("d2d3", 10),
            ("b1c3", 5),
        ]
    );

    // === ITALIAN GAME ===
    
    // After 1.e4 e5 2.Nf3 Nc6 3.Bc4
    add_position(&mut book,
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
        vec![
            ("f8c5", 50),  // Giuoco Piano
            ("g8f6", 40),  // Two Knights
            ("f8e7", 10),  // Hungarian
        ]
    );

    // Giuoco Piano
    add_position(&mut book,
        "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        vec![
            ("c2c3", 60),  // Main line
            ("d2d3", 25),  // Giuoco Pianissimo
            ("b2b4", 10),  // Evans Gambit
            ("e1g1", 5),
        ]
    );

    // === SICILIAN DEFENSE ===
    
    // After 1.e4 c5
    add_position(&mut book,
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 2",
        vec![
            ("g1f3", 85),  // Open Sicilian
            ("b1c3", 10),  // Closed Sicilian
            ("c2c3", 5),   // Alapin
        ]
    );

    // After 1.e4 c5 2.Nf3
    add_position(&mut book,
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",
        vec![
            ("d7d6", 40),  // Najdorf/Dragon setup
            ("b8c6", 30),  // Old Sicilian
            ("e7e6", 20),  // Paulsen/Taimanov
            ("g7g6", 10),  // Hyperaccelerated Dragon
        ]
    );

    // After 1.e4 c5 2.Nf3 d6
    add_position(&mut book,
        "rnbqkbnr/pp2pppp/3p4/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 3",
        vec![
            ("d2d4", 95),  // Main line
            ("f1b5", 5),   // Rossolimo delayed
        ]
    );

    // After 1.e4 c5 2.Nf3 d6 3.d4
    add_position(&mut book,
        "rnbqkbnr/pp2pppp/3p4/2p5/3PP3/5N2/PPP2PPP/RNBQKB1R b KQkq d3 0 3",
        vec![
            ("c5d4", 100),  // Take
        ]
    );

    // After 1.e4 c5 2.Nf3 d6 3.d4 cxd4
    add_position(&mut book,
        "rnbqkbnr/pp2pppp/3p4/8/3pP3/5N2/PPP2PPP/RNBQKB1R w KQkq - 0 4",
        vec![
            ("f3d4", 100),  // Recapture
        ]
    );

    // Open Sicilian after 4.Nxd4
    add_position(&mut book,
        "rnbqkbnr/pp2pppp/3p4/8/3NP3/8/PPP2PPP/RNBQKB1R b KQkq - 0 4",
        vec![
            ("g8f6", 60),  // Najdorf/Classical
            ("b8c6", 30),  // Classical
            ("g7g6", 10),  // Dragon
        ]
    );

    // === FRENCH DEFENSE ===
    
    // After 1.e4 e6
    add_position(&mut book,
        "rnbqkbnr/pppp1ppp/4p3/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
        vec![
            ("d2d4", 80),  // Main line
            ("d2d3", 15),  // King's Indian Attack
            ("b1c3", 5),
        ]
    );

    // After 1.e4 e6 2.d4
    add_position(&mut book,
        "rnbqkbnr/pppp1ppp/4p3/8/3PP3/8/PPP2PPP/RNBQKBNR b KQkq d3 0 2",
        vec![
            ("d7d5", 100),  // French proper
        ]
    );

    // After 1.e4 e6 2.d4 d5
    add_position(&mut book,
        "rnbqkbnr/ppp2ppp/4p3/3p4/3PP3/8/PPP2PPP/RNBQKBNR w KQkq d6 0 3",
        vec![
            ("b1c3", 50),  // Winawer/Classical
            ("e4d5", 30),  // Exchange
            ("e4e5", 20),  // Advance
        ]
    );

    // === QUEEN'S PAWN OPENINGS (1.d4) ===
    
    // After 1.d4
    add_position(&mut book,
        "rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq d3 0 1",
        vec![
            ("d7d5", 50),  // Queen's Gambit
            ("g8f6", 40),  // Indian Defenses
            ("e7e6", 8),   // French/Queen's Gambit
            ("f7f5", 2),   // Dutch
        ]
    );

    // After 1.d4 d5
    add_position(&mut book,
        "rnbqkbnr/ppp1pppp/8/3p4/3P4/8/PPP1PPPP/RNBQKBNR w KQkq d6 0 2",
        vec![
            ("c2c4", 85),  // Queen's Gambit
            ("g1f3", 10),  // London/Colle
            ("c1f4", 5),   // London System
        ]
    );

    // === QUEEN'S GAMBIT ===
    
    // After 1.d4 d5 2.c4
    add_position(&mut book,
        "rnbqkbnr/ppp1pppp/8/3p4/2PP4/8/PP2PPPP/RNBQKBNR b KQkq c3 0 2",
        vec![
            ("e7e6", 50),  // Queen's Gambit Declined
            ("d5c4", 25),  // Queen's Gambit Accepted
            ("c7c6", 20),  // Slav
            ("e7e5", 5),   // Albin Counter-Gambit
        ]
    );

    // Queen's Gambit Declined
    add_position(&mut book,
        "rnbqkbnr/ppp2ppp/4p3/3p4/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 3",
        vec![
            ("b1c3", 70),  // Main line
            ("g1f3", 25),
            ("c4d5", 5),
        ]
    );

    // After 3.Nc3
    add_position(&mut book,
        "rnbqkbnr/ppp2ppp/4p3/3p4/2PP4/2N5/PP2PPPP/R1BQKBNR b KQkq - 1 3",
        vec![
            ("g8f6", 85),  // Main line
            ("c7c6", 10),  // Semi-Slav
            ("f8e7", 5),
        ]
    );

    // === INDIAN DEFENSES ===
    
    // After 1.d4 Nf6
    add_position(&mut book,
        "rnbqkb1r/pppppppp/5n2/8/3P4/8/PPP1PPPP/RNBQKBNR w KQkq - 1 2",
        vec![
            ("c2c4", 75),  // Mainline systems
            ("g1f3", 20),  // London/Torre
            ("c1f4", 5),   // London System
        ]
    );

    // After 1.d4 Nf6 2.c4
    add_position(&mut book,
        "rnbqkb1r/pppppppp/5n2/8/2PP4/8/PP2PPPP/RNBQKBNR b KQkq c3 0 2",
        vec![
            ("e7e6", 45),  // Queen's Indian/Nimzo-Indian
            ("g7g6", 35),  // King's Indian
            ("e7e5", 15),  // Budapest
            ("c7c5", 5),   // Benoni
        ]
    );

    // Nimzo-Indian setup
    add_position(&mut book,
        "rnbqkb1r/pppp1ppp/4pn2/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 3",
        vec![
            ("b1c3", 90),  // Nimzo-Indian proper
            ("g1f3", 10),  // Queen's Indian
        ]
    );

    // Nimzo-Indian
    add_position(&mut book,
        "rnbqkb1r/pppp1ppp/4pn2/8/2PP4/2N5/PP2PPPP/R1BQKBNR b KQkq - 1 3",
        vec![
            ("f8b4", 85),  // Nimzo-Indian
            ("b7b6", 10),  // Queen's Indian
            ("d7d5", 5),
        ]
    );

    // King's Indian setup
    add_position(&mut book,
        "rnbqkb1r/pppppp1p/5np1/8/2PP4/8/PP2PPPP/RNBQKBNR w KQkq - 0 3",
        vec![
            ("b1c3", 70),  // Main line
            ("g1f3", 25),
            ("g2g3", 5),   // Fianchetto
        ]
    );

    // === ENGLISH OPENING ===
    
    // After 1.c4
    add_position(&mut book,
        "rnbqkbnr/pppppppp/8/8/2P5/8/PP1PPPPP/RNBQKBNR b KQkq c3 0 1",
        vec![
            ("e7e5", 35),  // Reversed Sicilian
            ("g8f6", 30),  // Various systems
            ("c7c5", 20),  // Symmetrical
            ("e7e6", 10),
            ("g7g6", 5),
        ]
    );

    // After 1.c4 e5
    add_position(&mut book,
        "rnbqkbnr/pppp1ppp/8/4p3/2P5/8/PP1PPPPP/RNBQKBNR w KQkq e6 0 2",
        vec![
            ("b1c3", 60),  // Main line
            ("g1f3", 30),
            ("g2g3", 10),
        ]
    );

    book
}

fn add_position(book: &mut HashMap<String, BookPosition>, fen: &str, moves: Vec<(&str, u32)>) {
    let mut position = BookPosition::new();
    for (mv, weight) in moves {
        position.add_move(mv, weight);
    }
    book.insert(fen.to_string(), position);
}

/// Probe the opening book for a move
pub fn probe_book(fen: &str) -> Option<String> {
    OPENING_BOOK.get(fen).and_then(|pos| pos.get_random_move())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starting_position() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mv = probe_book(fen);
        assert!(mv.is_some());
        
        let move_str = mv.unwrap();
        assert!(["e2e4", "d2d4", "c2c4", "g1f3", "g2g3"].contains(&move_str.as_str()));
    }

    #[test]
    fn test_unknown_position() {
        let fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        let mv = probe_book(fen);
        assert!(mv.is_none());
    }

    #[test]
    fn test_book_coverage() {
        // Test that major openings are covered
        let positions = vec![
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1", // After 1.e4
            "rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq d3 0 1",  // After 1.d4
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2", // After 1.e4 e5
        ];

        for fen in positions {
            assert!(probe_book(fen).is_some(), "Book missing position: {}", fen);
        }
    }
}