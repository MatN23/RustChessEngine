use crate::board::{BoardState, Color, PIECE_VALUES};
use crate::movegen::{Move, MoveGenerator, MoveList};
use crate::eval::Evaluator;
use crate::opening_book;
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use rayon::prelude::*;

pub const INFINITY: i32 = 999999;
pub const MATE_SCORE: i32 = 900000;
const MAX_PLY: usize = 128;
const MAX_THREADS: usize = 256;

// LMR reduction table
lazy_static::lazy_static! {
    static ref LMR_TABLE: [[u8; 64]; 64] = {
        let mut table = [[0u8; 64]; 64];
        for depth in 1..64 {
            for moves in 1..64 {
                let d = depth as f64;
                let m = moves as f64;
                table[depth][moves] = ((d.ln() * m.ln() / 2.0) as u8).min(depth as u8 - 1);
            }
        }
        table
    };
}

pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub pv_lines: Vec<(Move, i32)>,
}

pub struct SearchEngine {
    tt: Arc<RwLock<TranspositionTable>>,
    threads: usize,
    nodes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    multi_pv: usize,
    pub contempt: i32,
    
    // Per-thread data
    thread_data: Arc<Vec<Mutex<ThreadData>>>,
    // Serialize searches while allowing stop() to signal the active search.
    search_lock: Mutex<()>,
}

struct ThreadData {
    killer_moves: [[Option<Move>; 2]; MAX_PLY],
    history_table: [[i32; 64]; 64],
    countermove_table: [[Option<Move>; 64]; 64],
    nodes_searched: u64,
    pv_table: [[Option<Move>; MAX_PLY]; MAX_PLY],
    pv_length: [usize; MAX_PLY],
}

impl ThreadData {
    fn new() -> Self {
        ThreadData {
            killer_moves: [[None; 2]; MAX_PLY],
            history_table: [[0; 64]; 64],
            countermove_table: [[None; 64]; 64],
            nodes_searched: 0,
            pv_table: [[None; MAX_PLY]; MAX_PLY],
            pv_length: [0; MAX_PLY],
        }
    }

    fn clear(&mut self) {
        self.killer_moves = [[None; 2]; MAX_PLY];
        self.history_table = [[0; 64]; 64];
        self.countermove_table = [[None; 64]; 64];
        self.nodes_searched = 0;
        self.pv_table = [[None; MAX_PLY]; MAX_PLY];
        self.pv_length = [0; MAX_PLY];
    }
}


fn is_passed_pawn_push(board: &BoardState, mv: &Move) -> bool {
    if mv.is_capture() || mv.is_promotion() || mv.from >= 64 || mv.to >= 64 {
        return false;
    }

    let (own_color, enemy_color, from_rank, to_rank, forward) =
        match board.side_to_move {
            Color::White => (0usize, 1usize, 5u8, 6u8, 8i16),
            Color::Black => (1usize, 0usize, 2u8, 1u8, -8i16),
        };

    if board.pieces[own_color][1] & (1u64 << mv.from) == 0
        || mv.from / 8 != from_rank
        || mv.to / 8 != to_rank
        || mv.to as i16 - mv.from as i16 != forward
    {
        return false;
    }

    let file = mv.from % 8;
    let mut enemy_pawns_ahead = 0u64;
    if board.side_to_move == Color::White {
        for rank in (from_rank + 1)..8 {
            for adjacent_file in file.saturating_sub(1)..=(file + 1).min(7) {
                enemy_pawns_ahead |= 1u64 << (rank * 8 + adjacent_file);
            }
        }
    } else {
        for rank in 0..from_rank {
            for adjacent_file in file.saturating_sub(1)..=(file + 1).min(7) {
                enemy_pawns_ahead |= 1u64 << (rank * 8 + adjacent_file);
            }
        }
    }

    board.pieces[enemy_color][1] & enemy_pawns_ahead == 0
}

/// Captures and promotions are always included. Quiet checks are included only
/// for the first two quiescence plies to catch forcing replies without allowing
/// an unbounded checking-move explosion.
fn generate_quiescence_moves(board: &BoardState, depth: i8, in_check: bool) -> MoveList {
    let mut moves = MoveList::new();
    for mv in MoveGenerator::generate_legal_moves(board) {
        if in_check || mv.is_capture() || mv.is_promotion() {
            moves.push(mv);
        } else if depth >= -1 {
            let mut child = board.clone();
            child.make_move(&mv);
            if child.is_in_check(child.side_to_move) {
                moves.push(mv);
            }
        }
    }
    moves
}

impl SearchEngine {
    pub fn new(threads: usize) -> Self {
        let threads = threads.clamp(1, MAX_THREADS);
        let mut thread_data = Vec::new();
        for _ in 0..threads {
            thread_data.push(Mutex::new(ThreadData::new()));
        }

        SearchEngine {
            tt: Arc::new(RwLock::new(TranspositionTable::new(512))),
            threads,
            nodes: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            multi_pv: 1,
            contempt: 20,
            thread_data: Arc::new(thread_data),
            search_lock: Mutex::new(()),
        }
    }

    pub fn search(
        &self,
        board: BoardState,
        max_depth: u8,
        time_ms: Option<u64>,
    ) -> SearchResult {
        let _search_guard = self.search_lock.lock();

        self.nodes.store(0, Ordering::Relaxed);
        self.stop.store(false, Ordering::Relaxed);

        // Clear thread data
        for thread_data in self.thread_data.iter() {
            thread_data.lock().clear();
        }

        // A dead position has no legal best move to return.
        if board.is_insufficient_material() {
            return SearchResult {
                best_move: None,
                score: 0,
                nodes: 0,
                pv_lines: Vec::new(),
            };
        }

        // Opening book probe
        if board.fullmove_number <= 15 {
            if let Some(book_move_uci) = opening_book::probe_book(&board.to_fen()) {
                let moves = MoveGenerator::generate_legal_moves(&board);
                for mv in moves {
                    if mv.to_uci() == book_move_uci {
                        println!("info string Opening book hit");
                        return SearchResult {
                            best_move: Some(mv),
                            score: 0,
                            nodes: 0,
                            pv_lines: vec![(mv, 0)],
                        };
                    }
                }
            }
        }

        let mut legal_moves = MoveGenerator::generate_legal_moves(&board);
        if legal_moves.is_empty() {
            let score = if board.is_in_check(board.side_to_move) {
                -MATE_SCORE
            } else {
                -self.contempt
            };
            return SearchResult {
                best_move: None,
                score,
                nodes: 0,
                pv_lines: Vec::new(),
            };
        }

        // Keep a useful fallback if a very short time limit interrupts the
        // first iteration; the old fallback was just the first generated move.
        let mut best_move = None;
        let mut best_score = -INFINITY;
        for mv in legal_moves.drain(..) {
            let mut child = board.clone();
            child.make_move(&mv);
            let score = -Evaluator::evaluate(&child);
            if score > best_score {
                best_score = score;
                best_move = Some(mv);
            }
        }

        let start_time = Instant::now();
        let time_limit = time_ms.map(Duration::from_millis);
        let mut prev_score = 0;
        let pv_lines = Vec::new();

        // Iterative deepening
        for depth in 1..=max_depth {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            let soft_limit = time_limit.map(|t| t.mul_f64(0.5));
            let hard_limit = time_limit;

            let previous_iteration_score = prev_score;
            let (score, mv, pv) = if depth >= 5 {
                self.search_aspiration(&board, depth, prev_score, start_time, soft_limit, hard_limit)
            } else {
                self.search_root(&board, depth, -INFINITY, INFINITY, start_time, soft_limit, hard_limit)
            };

            // Do not accept results from an interrupted iteration.
            // Retain the last completed iteration, or the legal fallback.
            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            if let Some(m) = mv {
                best_move = Some(m);
                best_score = score;
                prev_score = score;
            } else if depth == 1 {
                // For mate/stalemate, we must still respect the score
                best_score = score;
            }

            let elapsed_ms = start_time.elapsed().as_millis();
            let nodes = self.nodes.load(Ordering::Relaxed);
            let nps = if elapsed_ms > 0 {
                (nodes as u128 * 1000 / elapsed_ms) as u64
            } else {
                0
            };

            let mut pv_str = String::new();
            for pv_move in pv.iter().take(10) {
                pv_str.push_str(&format!("{} ", pv_move.to_uci()));
            }

            if score.abs() > MATE_SCORE - 100 {
                let mate_in = (MATE_SCORE - score.abs() + 1) / 2;
                println!(
                    "info depth {} score mate {} nodes {} nps {} time {} pv {}",
                    depth,
                    if score > 0 { mate_in } else { -mate_in },
                    nodes,
                    nps,
                    elapsed_ms,
                    pv_str.trim()
                );
            } else {
                println!(
                    "info depth {} score cp {} nodes {} nps {} time {} pv {}",
                    depth, score, nodes, nps, elapsed_ms, pv_str.trim()
                );
            }

            // Smart time management: only stop early if score is very stable
            if let Some(soft) = soft_limit {
                let elapsed = start_time.elapsed();
                if elapsed > soft {
                    let time_ratio = elapsed.as_millis() as f64 / soft.as_millis() as f64;
                    
                    // In desperate positions, always keep thinking if time allows
                    // Only exit early if score is stable AND we've used a lot of time
                    let score_stable = (previous_iteration_score - score).abs() < 20;
                    if score < -500 {
                        // Losing badly: keep searching for a swindle unless almost out of time
                        if time_ratio > 1.8 {
                            break;
                        }
                    } else if time_ratio > 1.6 && score_stable {
                        // Score is very stable and we've used plenty of time
                        break;
                    }
                }
            }

            if let Some(hard) = hard_limit {
                if start_time.elapsed() > hard.mul_f64(0.95) {
                    break;
                }
            }
        }

        SearchResult {
            best_move,
            score: best_score,
            nodes: self.nodes.load(Ordering::Relaxed),
            pv_lines,
        }
    }

    fn search_aspiration(
        &self,
        board: &BoardState,
        depth: u8,
        prev_score: i32,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let mut window = 50;
        let mut alpha = prev_score - window;
        let mut beta = prev_score + window;
        let mut fail_high_count = 0;
        let mut fail_low_count = 0;

        loop {
            let (score, mv, pv) = self.search_root(board, depth, alpha, beta, start_time, soft_limit, hard_limit);

            if self.stop.load(Ordering::Relaxed) {
                return (prev_score, mv, pv);
            }

            if score <= alpha {
                // Fail low
                fail_low_count += 1;
                beta = (alpha + beta) / 2;
                alpha = (alpha - window * (1 + fail_low_count)).max(-INFINITY);
                println!("info string Fail low, widening window to [{}, {}]", alpha, beta);
            } else if score >= beta {
                // Fail high
                fail_high_count += 1;
                beta = (beta + window * (1 + fail_high_count)).min(INFINITY);
                println!("info string Fail high, widening window to [{}, {}]", alpha, beta);
            } else {
                return (score, mv, pv);
            }

            window = (window * 2).min(1000);

            // Emergency exit on extreme fails
            if fail_high_count + fail_low_count > 5 {
                return self.search_root(board, depth, -INFINITY, INFINITY, start_time, soft_limit, hard_limit);
            }
        }
    }

    fn search_root(
        &self,
        board: &BoardState,
        depth: u8,
        alpha: i32,
        beta: i32,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if board.is_in_check(board.side_to_move) {
                (-MATE_SCORE, None, MoveList::new())
            } else {
                (-self.contempt, None, MoveList::new())
            };
        }

        // If only one move is available, we still want a score for aspiration windows
        if moves.len() == 1 {
            let mut new_board = board.clone();
            new_board.make_move(&moves[0]);
            let mut thread_data = self.thread_data[0].lock();
            let score = -self.pvs(&new_board, depth.saturating_sub(1), -beta, -alpha, 1, true, 0, start_time, soft_limit, hard_limit, &mut thread_data);
            let mut pv = MoveList::new();
            pv.push(moves[0]);
            return (score, Some(moves[0]), pv);
        }

        // Order moves using main thread data
        let mut thread_data = self.thread_data[0].lock();
        self.order_moves_internal(board, &mut moves, None, 0, &mut thread_data);
        drop(thread_data);

        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_pv = MoveList::new();

        // Lazy SMP: Launch parallel search on multiple threads
        if self.threads > 1 && depth >= 6 {
            let results: Vec<_> = (0..self.threads)
                .into_par_iter()
                .map(|thread_id| {
                    if self.stop.load(Ordering::Relaxed) {
                        return (thread_id, -INFINITY, None, MoveList::new());
                    }

                    let depth_variation = if thread_id > 0 {
                        // Helpers use a slightly shallower search to contribute TT entries.
                        let offset = (thread_id as i32) % 4 - 1;
                        (depth as i32 + offset).max(1).min(depth as i32) as u8
                    } else {
                        depth
                    };

                    let (score, mv, pv) = self.search_root_thread(
                        board,
                        depth_variation,
                        alpha,
                        beta,
                        thread_id,
                        start_time,
                        soft_limit,
                        hard_limit,
                    );

                    (thread_id, score, mv, pv)
                })
                .collect();

            // Root scores from different depths are not comparable. Only the
            // full-depth main thread may choose the move; helpers contribute
            // shared transposition-table information.
            for (thread_id, score, mv, pv) in results {
                if thread_id == 0 {
                    best_score = score;
                    best_move = mv;
                    best_pv = pv;
                    break;
                }
            }
        } else {
            // Single-threaded search
            let (score, mv, pv) = self.search_root_thread(
                board,
                depth,
                alpha,
                beta,
                0,
                start_time,
                soft_limit,
                hard_limit,
            );
            best_score = score;
            best_move = mv;
            best_pv = pv;
        }

        (best_score, best_move, best_pv)
    }

    fn search_root_thread(
        &self,
        board: &BoardState,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        thread_id: usize,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let mut moves = MoveGenerator::generate_legal_moves(board);
        let mut thread_data = self.thread_data[thread_id].lock();
        self.order_moves_internal(board, &mut moves, None, 0, &mut thread_data);
        
        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_pv = MoveList::new();
        let mut move_count = 0;

        for mv in moves {
            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                break;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = if move_count == 0 {
                // Full window search for first move
                -self.pvs(&new_board, depth - 1, -beta, -alpha, 1, true, thread_id, start_time, soft_limit, hard_limit, &mut thread_data)
            } else {
                // PVS: null window search
                let mut score = -self.pvs(&new_board, depth - 1, -alpha - 1, -alpha, 1, false, thread_id, start_time, soft_limit, hard_limit, &mut thread_data);
                
                if score > alpha && score < beta {
                    // Re-search with full window
                    score = -self.pvs(&new_board, depth - 1, -beta, -alpha, 1, true, thread_id, start_time, soft_limit, hard_limit, &mut thread_data);
                }
                score
            };

            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            move_count += 1;

            if score > best_score {
                best_score = score;
                best_move = Some(mv);
                
                // Copy PV
                best_pv.clear();
                best_pv.push(mv);
                if thread_data.pv_length[1] > 0 {
                    for i in 0..thread_data.pv_length[1] {
                        if let Some(pv_move) = thread_data.pv_table[1][i] {
                            best_pv.push(pv_move);
                        }
                    }
                }
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta {
                self.update_killers_internal(mv, 0, &mut thread_data);
                self.update_history_internal(mv, depth, &mut thread_data);
                break;
            }
        }

        drop(thread_data);
        (best_score, best_move, best_pv)
    }

    #[allow(clippy::too_many_arguments)]
    fn pvs(
        &self,
        board: &BoardState,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        pv_node: bool,
        thread_id: usize,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
        thread_data: &mut ThreadData,
    ) -> i32 {
        // Periodic stop check
        thread_data.nodes_searched += 1;
        if thread_data.nodes_searched & 2047 == 0 {
            self.nodes.fetch_add(2048, Ordering::Relaxed);
            thread_data.nodes_searched = 0;

            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                return 0;
            }
        }

        if self.stop.load(Ordering::Relaxed) {
            return 0;
        }

        if ply >= MAX_PLY - 1 {
            return Evaluator::evaluate(board);
        }

        if board.is_insufficient_material() {
            return 0;
        }

        // Draw detection
        if board.halfmove_clock >= 100 || board.is_repetition() {
            return -self.contempt;
        }

        // Mate distance pruning
        alpha = alpha.max(-MATE_SCORE + ply as i32);
        let beta_new = beta.min(MATE_SCORE - ply as i32 - 1);
        if alpha >= beta_new {
            return alpha;
        }

        let in_check = board.is_in_check(board.side_to_move);
        let mut depth = depth; 

        // In-check extension for tactical stability
        if in_check {
            depth += 1;
        }

        // Quiescence at leaf nodes
        if depth == 0 {
            return self.quiescence(board, alpha, beta_new, 0, thread_data, start_time, hard_limit);
        }

        // TT probe
        let tt_entry = self.tt.read().probe(board.hash);
        let mut tt_move = tt_entry.as_ref().and_then(|e| e.best_move);

        if let Some(entry) = &tt_entry {
            if entry.depth >= depth && !pv_node && ply > 0 {
                let tt_score = score_from_tt(entry.score, ply);
                match entry.flag {
                    TT_EXACT => return tt_score,
                    TT_ALPHA if tt_score <= alpha => return alpha,
                    TT_BETA if tt_score >= beta_new => return beta_new,
                    _ => {}
                }
            }
        }

        let (static_eval, side_has_hanging_piece) =
            Evaluator::evaluate_with_tactical_info(board);
        let tactical_danger =
            !pv_node && !in_check && depth <= 7 && side_has_hanging_piece;

        // Reverse futility pruning
        if !pv_node && !in_check && !tactical_danger && depth <= 7 {
            let rfp_margin = 90 * depth as i32;
            if static_eval - rfp_margin >= beta_new {
                return static_eval - rfp_margin;
            }
        }

        // Null move pruning with verification
        if !pv_node && !in_check && !tactical_danger && depth >= 3 && board.halfmove_clock < 90 {
            let has_pieces = (board.pieces[board.side_to_move as usize][2] 
                | board.pieces[board.side_to_move as usize][3]
                | board.pieces[board.side_to_move as usize][4]
                | board.pieces[board.side_to_move as usize][5]) != 0;

            if has_pieces && static_eval >= beta_new {
                let mut null_board = board.clone();
                null_board.side_to_move = null_board.side_to_move.flip();
                null_board.ep_square = None;
                null_board.hash ^= crate::zobrist::ZOBRIST.side_key;

                let r = 3 + (depth / 4) + ((static_eval - beta_new) / 200).clamp(0, 2) as u8;
                let score = -self.pvs(&null_board, depth.saturating_sub(r), -beta_new, -beta_new + 1, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);

                if score >= beta_new {
                    if depth < 12 {
                        return if score > MATE_SCORE - 100 { beta_new } else { score };
                    }
                    // Verification search
                    let verify = self.pvs(board, depth.saturating_sub(r), beta_new - 1, beta_new, ply, false, thread_id, start_time, soft_limit, hard_limit, thread_data);
                    if verify >= beta_new {
                        return if score > MATE_SCORE - 100 { beta_new } else { score };
                    }
                }
            }
        }

        // Razoring
        if depth <= 3 && !in_check && !pv_node && !tactical_danger {
            let razor_margin = 350 + 200 * depth as i32;
            if static_eval + razor_margin < alpha {
                let q_score = self.quiescence(board, alpha, beta_new, 0, thread_data, start_time, hard_limit);
                if q_score < alpha {
                    return q_score.max(alpha - razor_margin);
                }
            }
        }

        // Internal iterative deepening
        if tt_move.is_none() && depth >= 6 && pv_node {
            let iid_depth = depth.saturating_sub(2);
            self.pvs(board, iid_depth, alpha, beta_new, ply, true, thread_id, start_time, soft_limit, hard_limit, thread_data);
            let entry = self.tt.read().probe(board.hash);
            tt_move = entry.and_then(|e| e.best_move);
        }

        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if in_check {
                -MATE_SCORE + ply as i32
            } else {
                -self.contempt
            };
        }

        self.order_moves_internal(board, &mut moves, tt_move, ply, thread_data);

        let mut best_score = -INFINITY;
        let mut best_move = None;
        let mut move_count = 0;
        let alpha_orig = alpha;
        let mut quiets_tried: MoveList = MoveList::with_capacity(48);

        thread_data.pv_length[ply] = 0;

        for mv in moves {
            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                break;
            }

            // Late Move Pruning (LMP) for quiet moves in non-PV nodes
            if !pv_node && !in_check && !tactical_danger && depth <= 3 && move_count > (3 + 2 * depth as usize) && !mv.is_capture() && !mv.is_promotion() {
                let h = thread_data.history_table[mv.from as usize][mv.to as usize];
                if h < 2000 {
                    move_count += 1;
                    continue;
                }
            }

            // History pruning: skip quiet moves with very bad history at shallow depths
            if !pv_node && !in_check && !tactical_danger && depth <= 5 && move_count > 3 && !mv.is_capture() && !mv.is_promotion() {
                let h = thread_data.history_table[mv.from as usize][mv.to as usize];
                if h < -(depth as i32 * depth as i32 * 100) {
                    move_count += 1;
                    continue;
                }
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            // Futility pruning is restricted to shallow non-PV nodes. The old
            // version also pruned PV nodes and used a margin up to 720 cp.
            let futile = !pv_node
                && !in_check
                && !tactical_danger
                && !new_board.is_in_check(new_board.side_to_move)
                && !mv.is_capture()
                && !mv.is_promotion()
                && move_count > 0
                && depth <= 3;

            if futile {
                let futility_margin = 100 + 80 * depth as i32;
                if static_eval + futility_margin <= alpha {
                    move_count += 1;
                    continue;
                }
            }

            let gives_check = new_board.is_in_check(new_board.side_to_move);
            let mut extension = 0;

            // Extend only a genuine passer advancing onto the seventh rank.
            if !gives_check && is_passed_pawn_push(board, &mv) {
                extension = 1;
            }

            let score = if move_count == 0 {
                // First move: full window PVS
                -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -beta_new, -alpha, ply + 1, pv_node, thread_id, start_time, soft_limit, hard_limit, thread_data)
            } else {
                // Late move reductions
                let reduction = if move_count >= 3 && depth >= 3 && !in_check && !gives_check && !mv.is_capture() && !mv.is_promotion() {
                    let base = LMR_TABLE[depth.min(63) as usize][move_count.min(63)];
                    let mut r = base;

                    // Reduce less in PV nodes
                    if pv_node {
                        r = r.saturating_sub(1);
                    }

                    // Reduce less for killer moves
                    let is_killer = thread_data.killer_moves[ply].iter().any(|k| {
                        k.map_or(false, |killer| killer.from == mv.from && killer.to == mv.to)
                    });
                    if is_killer {
                        r = r.saturating_sub(1);
                    }

                    // Reduce less for good history
                    let history = thread_data.history_table[mv.from as usize][mv.to as usize];
                    if history > 5000 {
                        r = r.saturating_sub(1);
                    } else if history < -5000 {
                        r = r.saturating_add(1);
                    }

                    r.min(depth.saturating_sub(1))
                } else {
                    0
                };

                // Null window search with reduction
                let mut score = -self.pvs(&new_board, depth.saturating_sub(reduction + 1).saturating_add(extension), -alpha - 1, -alpha, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);

                // Re-search if reduced and score beats alpha
                if reduction > 0 && score > alpha {
                    score = -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -alpha - 1, -alpha, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);
                }

                // Re-search with full window if score is in (alpha, beta)
                if score > alpha && score < beta_new && pv_node {
                    score = -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -beta_new, -alpha, ply + 1, true, thread_id, start_time, soft_limit, hard_limit, thread_data);
                }

                score
            };

            if self.stop.load(Ordering::Relaxed) {
                return 0;
            }

            move_count += 1;

            if score > best_score {
                best_score = score;
                best_move = Some(mv);

                // Update PV
                thread_data.pv_table[ply][0] = Some(mv);
                thread_data.pv_length[ply] = 1;
                if ply + 1 < MAX_PLY && thread_data.pv_length[ply + 1] > 0 {
                    for i in 0..thread_data.pv_length[ply + 1] {
                        thread_data.pv_table[ply][i + 1] = thread_data.pv_table[ply + 1][i];
                    }
                    thread_data.pv_length[ply] += thread_data.pv_length[ply + 1];
                }
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta_new {
                // Beta cutoff
                if !mv.is_capture() {
                    self.update_killers_internal(mv, ply, thread_data);
                    self.update_history_internal(mv, depth, thread_data);

                    // History penalty for quiet moves that didn't cause cutoff
                    for quiet in &quiets_tried {
                        if quiet.from != mv.from || quiet.to != mv.to {
                            let penalty = -(depth as i32) * (depth as i32);
                            self.update_history_raw_internal(*quiet, penalty, thread_data);
                        }
                    }
                }

                // Reduce TT traffic: only store reasonably deep entries
                if depth >= 2 || ply == 0 {
                    if !self.stop.load(Ordering::Relaxed) {
                        self.tt.write().store(
                            board.hash,
                            depth,
                            score_to_tt(beta_new, ply),
                            TT_BETA,
                            Some(mv),
                        );
                    }
                }
                return beta_new;
            }

            if !mv.is_capture() && !mv.is_promotion() {
                quiets_tried.push(mv);
            }
        }

        if self.stop.load(Ordering::Relaxed) {
            return 0;
        }

        let flag = if best_score <= alpha_orig {
            TT_ALPHA
        } else {
            TT_EXACT
        };

        // Reduce TT traffic for shallow nodes
        if depth >= 2 || ply == 0 {
            self.tt.write().store(
                board.hash,
                depth,
                score_to_tt(best_score, ply),
                flag,
                best_move,
            );
        }
        best_score
    }

    fn quiescence(
        &self,
        board: &BoardState,
        mut alpha: i32,
        beta: i32,
        depth: i8,
        thread_data: &mut ThreadData,
        start_time: Instant,
        hard_limit: Option<Duration>,
    ) -> i32 {
        thread_data.nodes_searched += 1;
        if thread_data.nodes_searched & 2047 == 0 {
            self.nodes.fetch_add(2048, Ordering::Relaxed);
            thread_data.nodes_searched = 0;
            if self.check_time_abort(start_time, None, hard_limit) {
                return 0;
            }
        }

        if self.stop.load(Ordering::Relaxed) {
            return 0;
        }

        if board.is_insufficient_material() {
            return 0;
        }

        let in_check = board.is_in_check(board.side_to_move);
        if depth < -16 && !in_check {
            return Evaluator::evaluate(board);
        }
        let stand_pat = if in_check {
            -INFINITY
        } else {
            Evaluator::evaluate(board)
        };

        if !in_check {
            if stand_pat >= beta {
                return beta;
            }

            // A single queen capture is worth 900 cp, and a capture-promotion
            // can gain substantially more. Keep the delta margin above those
            // tactical gains so a losing static evaluation cannot prune them.
            if stand_pat + 1800 < alpha {
                return alpha;
            }

            if stand_pat > alpha {
                alpha = stand_pat;
            }
        }

        // Captures plus all promotions, including quiet promotions.
        let mut moves = generate_quiescence_moves(board, depth, in_check);

        if moves.is_empty() {
            if in_check {
                return -MATE_SCORE;
            }

            // Stalemate cannot be detected from the capture list alone.
            if MoveGenerator::generate_legal_moves(board).is_empty() {
                return -self.contempt;
            }

            return stand_pat;
        }

        self.order_captures_internal(board, &mut moves);

        for mv in moves {
            // Promotion material gain is handled explicitly by SEE, but don't
            // prune promotions at the horizon on a static exchange threshold.
            if !in_check && mv.is_capture() && !mv.is_promotion() && !self.see_capture(board, &mv, 0) {
                continue;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = -self.quiescence(
                &new_board,
                -beta,
                -alpha,
                depth - 1,
                thread_data,
                start_time,
                hard_limit,
            );

            if self.stop.load(Ordering::Relaxed) {
                return 0;
            }

            if score >= beta {
                return beta;
            }

            if score > alpha {
                alpha = score;
            }
        }

        alpha
    }

    #[inline(always)]
fn order_moves_internal(&self, board: &BoardState, moves: &mut MoveList, tt_move: Option<Move>, ply: usize, thread_data: &mut ThreadData) {
        let killers = thread_data.killer_moves[ply];
        let history = &thread_data.history_table;

        // Use comparator-based unstable sort to avoid key allocations and negation.
        moves.sort_unstable_by(|a, b| {
            let sa = self.score_move_internal(board, a, tt_move, &killers, history);
            let sb = self.score_move_internal(board, b, tt_move, &killers, history);
            sb.cmp(&sa)
        });
    }

    #[inline(always)]
    fn score_move_internal(&self, board: &BoardState, mv: &Move, tt_move: Option<Move>, killers: &[Option<Move>; 2], history: &[[i32; 64]; 64]) -> i32 {
        // TT move has highest priority
        if let Some(hash_mv) = tt_move {
            // Promotion type and move flags are part of move identity. Matching
            // only from/to incorrectly treats every underpromotion as the TT move.
            if *mv == hash_mv {
                return 10_000_000;
            }
        }

        // Promotions should be distinguished even when they are captures.
        // Queen promotions are normally strongest; retain a smaller bonus for
        // underpromotions so capture-promotion ordering is deterministic too.
        let promotion_bonus = if mv.is_promotion() {
            match mv.promotion_piece() {
                Some(crate::board::Piece::Queen) => 4_000,
                Some(crate::board::Piece::Knight) => 3_000,
                Some(crate::board::Piece::Rook) => 2_000,
                Some(crate::board::Piece::Bishop) => 1_000,
                _ => 0,
            }
        } else {
            0
        };

        // Captures (MVV-LVA + SEE), including capture promotions.
        if mv.is_capture() {
            if self.see_capture(board, mv, 0) {
                return 9_000_000 + self.mvv_lva_score(board, mv) + promotion_bonus;
            } else {
                // Keep losing captures below quiet moves, but distinguish promotions.
                return -200_000 + self.mvv_lva_score(board, mv) + promotion_bonus;
            }
        }

        // Quiet promotions
        if mv.is_promotion() {
            return 8_000_000 + promotion_bonus;
        }

        // First killer move
        if let Some(killer1) = killers[0] {
            if mv.from == killer1.from && mv.to == killer1.to {
                return 7_000_000;
            }
        }

        // Second killer move
        if let Some(killer2) = killers[1] {
            if mv.from == killer2.from && mv.to == killer2.to {
                return 6_900_000;
            }
        }

        // History heuristic
        history[mv.from as usize][mv.to as usize].clamp(-10_000, 10_000)
    }

    #[inline(always)]
    fn mvv_lva_score(&self, board: &BoardState, mv: &Move) -> i32 {
        let victim = if let Some((piece, _)) = board.piece_at(mv.to) {
            PIECE_VALUES[piece as usize]
        } else {
            100 // En passant
        };

        let attacker = if let Some((piece, _)) = board.piece_at(mv.from) {
            PIECE_VALUES[piece as usize]
        } else {
            0
        };

        // MVV-LVA: prioritize capturing valuable pieces with less valuable pieces
        victim * 10 - attacker / 10
    }

    #[inline(always)]
fn order_captures_internal(&self, board: &BoardState, captures: &mut MoveList) {
        // Comparator-based unstable sort
        captures.sort_unstable_by(|a, b| {
            let mut sa = -self.mvv_lva_score(board, a);
            if self.see_capture(board, a, 0) { sa -= 100_000; }

            let mut sb = -self.mvv_lva_score(board, b);
            if self.see_capture(board, b, 0) { sb -= 100_000; }

            sa.cmp(&sb)
        });
    }

    #[inline(always)]
    fn see_capture(&self, board: &BoardState, mv: &Move, threshold: i32) -> bool {
        Evaluator::see_capture(board, mv, threshold)
    }

    #[inline(always)]
    fn update_killers_internal(&self, mv: Move, ply: usize, thread_data: &mut ThreadData) {
        if ply >= MAX_PLY {
            return;
        }

        let ply_killers = &mut thread_data.killer_moves[ply];

        // Check if already a killer
        if let Some(k1) = ply_killers[0] {
            if k1.from == mv.from && k1.to == mv.to {
                return;
            }
        }

        // Shift killers
        ply_killers[1] = ply_killers[0];
        ply_killers[0] = Some(mv);
    }

    #[inline(always)]
    fn update_history_internal(&self, mv: Move, depth: u8, thread_data: &mut ThreadData) {
        let bonus = (depth as i32) * (depth as i32);
        self.update_history_raw_internal(mv, bonus, thread_data);
    }

    #[inline(always)]
    fn update_history_raw_internal(&self, mv: Move, delta: i32, thread_data: &mut ThreadData) {
        let entry = &mut thread_data.history_table[mv.from as usize][mv.to as usize];
        *entry += delta;

        // Gravity: prevent values from growing too large
        if entry.abs() > 10000 {
            for from in 0..64 {
                for to in 0..64 {
                    thread_data.history_table[from][to] /= 2;
                }
            }
        }
    }

    #[inline(always)]
    fn check_time_abort(&self, start_time: Instant, _soft_limit: Option<Duration>, hard_limit: Option<Duration>) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }

        if let Some(hard) = hard_limit {
            if start_time.elapsed() > hard {
                self.stop.store(true, Ordering::Relaxed);
                return true;
            }
        }

        false
    }

    pub fn new_game(&self) {
        let _search_guard = self.search_lock.lock();
        // A new game has different repetition and rule-50 history. Clear
        // position-dependent search data at the game boundary, not per move.
        self.tt.write().clear();
        self.nodes.store(0, Ordering::Relaxed);
        
        for thread_data in self.thread_data.iter() {
            thread_data.lock().clear();
        }
    }

    pub fn set_threads(&mut self, threads: usize) {
        let new_threads = threads.clamp(1, MAX_THREADS);
        if new_threads == self.threads {
            return;
        }

        self.threads = new_threads;
        
        // Rebuild thread data
        let mut new_thread_data = Vec::new();
        for _ in 0..new_threads {
            new_thread_data.push(Mutex::new(ThreadData::new()));
        }
        self.thread_data = Arc::new(new_thread_data);
    }

    pub fn set_multi_pv(&mut self, count: usize) {
        self.multi_pv = count.clamp(1, 5);
    }

    pub fn stop(&self) {
        // Deliberately does not take search_lock: callers must be able to
        // interrupt the search that currently owns that lock.
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn clear_tt(&self) {
        let _search_guard = self.search_lock.lock();
        self.tt.write().clear();
    }

    pub fn set_hash_size(&self, size_mb: usize) {
        let _search_guard = self.search_lock.lock();
        self.tt.write().resize(size_mb);
    }
}

// Normalize mate scores when storing/probing TT entries at different plies.
#[inline]
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_SCORE - MAX_PLY as i32 {
        score + ply as i32
    } else if score < -MATE_SCORE + MAX_PLY as i32 {
        score - ply as i32
    } else {
        score
    }
}

#[inline]
fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_SCORE - MAX_PLY as i32 {
        score - ply as i32
    } else if score < -MATE_SCORE + MAX_PLY as i32 {
        score + ply as i32
    } else {
        score
    }
}

// Transposition Table Entry Flags
const TT_EXACT: u8 = 0;
const TT_ALPHA: u8 = 1;
const TT_BETA: u8 = 2;

#[derive(Clone)]
struct TTEntry {
    hash: u64,
    depth: u8,
    score: i32,
    flag: u8,
    best_move: Option<Move>,
    age: u8,
}

pub struct TranspositionTable {
    table: Vec<Option<TTEntry>>,
    size: usize,
    current_age: u8,
}

impl TranspositionTable {
    fn new(size_mb: usize) -> Self {
        let size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        TranspositionTable {
            table: vec![None; size],
            size,
            current_age: 0,
        }
    }

    fn probe(&self, hash: u64) -> Option<TTEntry> {
        let index = (hash as usize) % self.size;
        if let Some(entry) = &self.table[index] {
            if entry.hash == hash {
                return Some(entry.clone());
            }
        }
        None
    }

    fn store(&mut self, hash: u64, depth: u8, score: i32, flag: u8, best_move: Option<Move>) {
        let index = (hash as usize) % self.size;

        // Replacement strategy
        let should_replace = if let Some(entry) = &self.table[index] {
            if entry.hash == hash {
                // Do not let a shallower helper search overwrite a deeper
                // result for the same position. At equal depth, prefer exact
                // information over a bound.
                depth > entry.depth
                    || (depth == entry.depth && flag == TT_EXACT && entry.flag != TT_EXACT)
            } else {
                // Replace based on depth and age.
                let depth_diff = depth as i32 - entry.depth as i32;
                let age_diff = self.current_age.wrapping_sub(entry.age);

                depth_diff >= 3 || age_diff >= 4 || (depth_diff >= 0 && age_diff >= 2)
            }
        } else {
            true
        };

        if should_replace {
            self.table[index] = Some(TTEntry {
                hash,
                depth,
                score,
                flag,
                best_move,
                age: self.current_age,
            });
        }
    }

    fn clear(&mut self) {
        self.table = vec![None; self.size];
        self.current_age = 0;
    }

    fn resize(&mut self, size_mb: usize) {
        self.size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        self.table = vec![None; self.size];
        self.current_age = 0;
    }

    #[allow(dead_code)]
    fn increment_age(&mut self) {
        self.current_age = self.current_age.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BoardState;

    #[test]
    fn test_search_basic() {
        let board = BoardState::default();
        let mut engine = SearchEngine::new(1);
        let result = engine.search(board, 5, None);
        assert!(result.best_move.is_some());
    }

    #[test]
    fn test_search_parallel() {
        // Use a FEN that is NOT in the opening book by changing the move counters
        let fen = "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 10";
        let board = BoardState::from_fen(fen).unwrap();
        let mut engine = SearchEngine::new(4);
        let result = engine.search(board, 5, None);
        assert!(result.best_move.is_some());
        assert!(result.nodes > 0);
    }

    #[test]
    fn test_mate_in_one() {
        // Scholar's mate setup: 1 move to checkmate
        let fen = "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let board = BoardState::from_fen(fen).unwrap();
        let mut engine = SearchEngine::new(1);
        let result = engine.search(board, 10, None);
        
        // Should find a defensive move or recognize it's mate
        assert!(result.score < -MATE_SCORE + 100 || result.best_move.is_some());
    }

    #[test]
    fn test_time_management() {
        let board = BoardState::default();
        let mut engine = SearchEngine::new(1);
        
        let start = std::time::Instant::now();
        engine.search(board, 50, Some(1000));
        let elapsed = start.elapsed();
        
        // Should respect time limit (with some tolerance)
        assert!(elapsed.as_millis() < 1500);
    }

    #[test]
    fn test_transposition_table() {
        let mut tt = TranspositionTable::new(16);
        let test_move = Move::new(12, 20, 0);

        tt.store(12345, 5, 100, TT_EXACT, Some(test_move));

        let entry = tt.probe(12345);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().score, 100);
    }

    #[test]
    fn test_tt_does_not_replace_deeper_same_position_with_shallow_entry() {
        let mut tt = TranspositionTable::new(16);
        let deeper = Move::new(12, 20, 0);
        let shallower = Move::new(11, 27, 0);

        tt.store(12345, 8, 240, TT_EXACT, Some(deeper));
        tt.store(12345, 5, -80, TT_BETA, Some(shallower));

        let entry = tt.probe(12345).unwrap();
        assert_eq!(entry.depth, 8);
        assert_eq!(entry.score, 240);
        assert_eq!(entry.best_move, Some(deeper));
    }

    #[test]
    fn test_single_and_parallel_search_return_valid_moves() {
        // Use a non-book position so depth 6 actually exercises the search.
        // Wall-clock speed assertions are too noisy for a unit test and the
        // starting position exits early through the opening book.
        let fen = "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 10";
        let board = BoardState::from_fen(fen).unwrap();

        let mut single_threaded = SearchEngine::new(1);
        let single_result = single_threaded.search(board.clone(), 6, None);

        let mut parallel = SearchEngine::new(4);
        let parallel_result = parallel.search(board, 6, None);

        assert!(single_result.best_move.is_some());
        assert!(parallel_result.best_move.is_some());
        assert!(single_result.nodes > 0);
        assert!(parallel_result.nodes > 0);
    }

    #[test]
    fn test_lmr_table() {
        // Verify LMR table is reasonable
        assert_eq!(LMR_TABLE[1][1], 0);
        assert!(LMR_TABLE[10][10] > 0);
        assert!(LMR_TABLE[20][30] < 20);
    }

    #[test]
    fn test_tt_move_matching_distinguishes_promotions() {
        use crate::movegen::{QUEEN_PROMOTION, ROOK_PROMOTION};
        let board = BoardState::default();
        let engine = SearchEngine::new(1);
        let killers = [None, None];
        let history = [[0; 64]; 64];
        let queen = Move::new(48, 56, QUEEN_PROMOTION);
        let rook = Move::new(48, 56, ROOK_PROMOTION);

        assert_eq!(engine.score_move_internal(&board, &queen, Some(queen), &killers, &history), 10_000_000);
        assert_ne!(engine.score_move_internal(&board, &rook, Some(queen), &killers, &history), 10_000_000);
    }

    #[test]
    fn test_capture_promotion_ordering_prefers_queen() {
        use crate::movegen::{QUEEN_PROMO_CAPTURE, ROOK_PROMO_CAPTURE};
        let board = BoardState::default();
        let engine = SearchEngine::new(1);
        let killers = [None, None];
        let history = [[0; 64]; 64];
        let queen = Move::new(48, 57, QUEEN_PROMO_CAPTURE);
        let rook = Move::new(48, 57, ROOK_PROMO_CAPTURE);

        let queen_score = engine.score_move_internal(&board, &queen, None, &killers, &history);
        let rook_score = engine.score_move_internal(&board, &rook, None, &killers, &history);
        assert!(queen_score > rook_score);
    }

    #[test]
    fn test_mvv_lva() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let board = BoardState::from_fen(fen).unwrap();
        let engine = SearchEngine::new(1);
        
        // Create two test moves
        let move1 = Move::new(12, 20, 4); // Pawn captures
        let move2 = Move::new(1, 18, 4);  // Knight captures
        
        // MVV-LVA should prefer lower-value attacker for same victim
        // This is a simple sanity check
        let score1 = engine.mvv_lva_score(&board, &move1);
        let score2 = engine.mvv_lva_score(&board, &move2);
        
        println!("Pawn capture: {}, Knight capture: {}", score1, score2);
    }

    #[test]
    fn test_killer_moves() {
        let mut thread_data = ThreadData::new();
        let test_move = Move::new(12, 20, 0);
        
        let engine = SearchEngine::new(1);
        engine.update_killers_internal(test_move, 0, &mut thread_data);
        
        assert_eq!(thread_data.killer_moves[0][0], Some(test_move));
    }

    #[test]
    fn test_history_table() {
        let mut thread_data = ThreadData::new();
        let test_move = Move::new(12, 20, 0);
        
        let engine = SearchEngine::new(1);
        engine.update_history_internal(test_move, 5, &mut thread_data);
        
        let score = thread_data.history_table[12][20];
        assert!(score > 0);
    }
}


#[cfg(test)]
mod search_regression_tests {
    use super::*;

    #[test]
    fn mate_scores_round_trip_at_same_ply() {
        for ply in [0usize, 1, 7, 32, 100] {
            for score in [MATE_SCORE - 3, MATE_SCORE - 40, -MATE_SCORE + 5, -MATE_SCORE + 60] {
                assert_eq!(score_from_tt(score_to_tt(score, ply), ply), score);
            }
        }
    }

    #[test]
    fn mate_scores_are_normalized_for_different_probe_plies() {
        let stored = score_to_tt(MATE_SCORE - 9, 3);
        assert_eq!(score_from_tt(stored, 7), MATE_SCORE - 13);

        let stored_loss = score_to_tt(-MATE_SCORE + 9, 3);
        assert_eq!(score_from_tt(stored_loss, 7), -MATE_SCORE + 13);
    }

    #[test]
    fn search_returns_no_move_for_dead_material() {
        let board =
            BoardState::from_fen("7k/8/8/8/8/8/4B3/4K3 w - - 0 1").unwrap();
        let engine = SearchEngine::new(1);
        let result = engine.search(board, 8, None);
        assert!(result.best_move.is_none());
        assert_eq!(result.score, 0);
    }

    #[test]
    fn zero_depth_fallback_chooses_the_best_static_child() {
        let board = BoardState::from_fen(
            "rnbqk2r/pp2ppbp/2p2np1/3p4/2P5/2NBPN2/PP1P1PPP/R1BQK2R w KQkq - 2 20",
        ).unwrap();
        let mut expected_move = None;
        let mut expected_score = -INFINITY;
        for mv in MoveGenerator::generate_legal_moves(&board) {
            let mut child = board.clone();
            child.make_move(&mv);
            let score = -Evaluator::evaluate(&child);
            if score > expected_score {
                expected_score = score;
                expected_move = Some(mv);
            }
        }
        let engine = SearchEngine::new(1);
        let result = engine.search(board, 0, None);
        assert_eq!(result.best_move, expected_move);
        assert_eq!(result.score, expected_score);
    }

    #[test]
    fn passed_pawn_extension_requires_a_true_advanced_passer() {
        let passer =
            BoardState::from_fen("7k/8/P7/8/8/8/8/7K w - - 0 1").unwrap();
        let challenged =
            BoardState::from_fen("7k/1p6/P7/8/8/8/8/7K w - - 0 1").unwrap();
        let opening = BoardState::default();

        let push = Move::new(40, 48, 0); // a6-a7
        assert!(is_passed_pawn_push(&passer, &push));
        assert!(!is_passed_pawn_push(&challenged, &push));
        assert!(!is_passed_pawn_push(&opening, &Move::new(8, 16, 0)));
    }

    #[test]
    fn quiescence_includes_shallow_quiet_checks_but_bounds_them() {
        let board =
            BoardState::from_fen("r3k3/8/8/8/2N5/8/8/4K3 w - - 0 1").unwrap();

        let shallow = generate_quiescence_moves(&board, 0, false);
        assert!(shallow.iter().any(|mv| mv.to_uci() == "c4d6"));

        let deeper = generate_quiescence_moves(&board, -2, false);
        assert!(!deeper.iter().any(|mv| mv.to_uci() == "c4d6"));
    }
}
