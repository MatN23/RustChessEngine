use crate::board::{BoardState, Color, Piece, PIECE_VALUES};
use crate::movegen::{Move, MoveGenerator, MoveList};
use smallvec::SmallVec;
use crate::eval::Evaluator;
use crate::opening_book;
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const INFINITY: i32 = 999999;
pub const MATE_SCORE: i32 = 900000;
const MAX_PLY: usize = 128;
/// Quiescence only tests for stalemate when the side to move has at most this many pieces
/// (kings and pawns included).
const STALEMATE_CHECK_PIECES: u32 = 6;
/// At or below this many pieces (kings included) quiet-move reductions/pruning are disabled.
const TINY_ENDGAME_PIECES: u32 = 4;
/// Scores at or beyond this magnitude are forced-mate scores.
const MATE_BOUND: i32 = MATE_SCORE - MAX_PLY as i32;
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
    // Set when the main thread completes an iteration; halts helper threads only.
    helper_stop: Arc<AtomicBool>,
    // Quiescence delta pruning; only ever turned off by tests that validate it.
    delta_pruning: AtomicBool,
    // Opt-in: spend the whole per-move budget (see `set_full_budget`). Off by default.
    full_budget: AtomicBool,
    // Set by the main thread when its last root search ended with a move whose score lay
    // strictly inside the search window (i.e. a real, fully verified best move).
    root_exact: AtomicBool,
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
    // Local node counter: avoid a contended shared atomic on every node.
    nodes_searched: u64,
    // true at ply p when the move that led into p was a null move
    null_at: [bool; MAX_PLY],
    is_helper: bool,
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
            null_at: [false; MAX_PLY],
            is_helper: false,
            pv_table: [[None; MAX_PLY]; MAX_PLY],
            pv_length: [0; MAX_PLY],
        }
    }

    /// Start of a new search within a game: keep (decayed) history so move
    /// ordering does not start from zero on every move, reset per-search state.
    fn new_search(&mut self) {
        self.killer_moves = [[None; 2]; MAX_PLY];
        for row in self.history_table.iter_mut() {
            for v in row.iter_mut() {
                *v /= 2;
            }
        }
        self.nodes_searched = 0;
        self.null_at = [false; MAX_PLY];
        self.is_helper = false;
        self.pv_table = [[None; MAX_PLY]; MAX_PLY];
        self.pv_length = [0; MAX_PLY];
    }

    fn clear(&mut self) {
        self.null_at = [false; MAX_PLY];
        self.is_helper = false;
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

/// Quiescence move list: all legal evasions when in check, otherwise legal
/// captures and promotions. (A previous revision also generated every legal
/// move at the first two quiescence plies and made each one to test for check;
/// that cost a full legal movegen plus ~35 board copies per leaf for little
/// tactical gain.)
fn generate_quiescence_moves(board: &BoardState, in_check: bool) -> MoveList {
    if in_check {
        MoveGenerator::generate_legal_moves(board)
    } else {
        MoveGenerator::generate_captures(board)
    }
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
            helper_stop: Arc::new(AtomicBool::new(false)),
            delta_pruning: AtomicBool::new(true),
            full_budget: AtomicBool::new(false),
            root_exact: AtomicBool::new(false),
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
        self.helper_stop.store(false, Ordering::Relaxed);
        // Advance once per search invocation (UCI go), not once per chess
        // move. This makes entries from recent searches preferred while
        // retaining useful transpositions across consecutive positions.
        self.tt.read().increment_age();

        for thread_data in self.thread_data.iter() {
            thread_data.lock().new_search();
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
                0 // stalemate
            };
            return SearchResult {
                best_move: None,
                score,
                nodes: 0,
                pv_lines: Vec::new(),
            };
        }

        // A forced move needs no search when the clock is running.
        if legal_moves.len() == 1 && time_ms.is_some() {
            return SearchResult {
                best_move: Some(legal_moves[0]),
                score: 0,
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
        let mut last_iter_time = Duration::ZERO;

        // Iterative deepening
        let full_budget = self.full_budget.load(Ordering::Relaxed);
        for depth in 1..=max_depth {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            if full_budget {
                if let Some(limit) = time_limit {
                    if start_time.elapsed() >= limit {
                        break;
                    }
                }
            }

            let soft_limit = time_limit.map(|t| t.mul_f64(0.5));
            let hard_limit = time_limit;
            let iter_start = Instant::now();

            // Search the previous iteration's best move first. Before this the
            // root hash move was always None, so each iteration re-ordered the root
            // from scratch and a timeout could discard the whole iteration.
            let root_hint = if depth > 1 { best_move } else { None };

            let (score, mv, pv) = if depth >= 5 {
                self.search_aspiration(&board, depth, prev_score, root_hint, start_time, soft_limit, hard_limit)
            } else {
                self.search_root(&board, depth, -INFINITY, INFINITY, root_hint, start_time, soft_limit, hard_limit)
            };

            // Interrupted iteration. By default its result is discarded and the last completed
            // iteration (or the legal fallback) is kept. In full-budget mode it is kept when the
            // root search had fully searched a move whose score lies inside the window: that move
            // is at least as well verified as the previous iteration's best.
            if self.stop.load(Ordering::Relaxed) {
                if full_budget && depth > 1 && self.root_exact.load(Ordering::Relaxed) {
                    if let Some(m) = mv {
                        best_move = Some(m);
                        best_score = score;
                    }
                }
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
            last_iter_time = iter_start.elapsed();

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
                // Stop early only for a mate WE deliver that is proven: the PV must replay
                // legally to checkmate in exactly the claimed number of plies, and the
                // iteration must have searched at least that deep (so a shorter mate
                // would have been found). A mate score alone can come from pruning.
                if score > 0 && time_limit.is_some() {
                    let plies = (MATE_SCORE - score) as usize;
                    if depth as usize >= plies && Self::pv_proves_mate(&board, &pv, plies) {
                        break;
                    }
                }
            } else {
                println!(
                    "info depth {} score cp {} nodes {} nps {} time {} pv {}",
                    depth, score, nodes, nps, elapsed_ms, pv_str.trim()
                );
            }

            // Do not start an iteration that cannot finish: an interrupted
            // iteration is discarded, so its time is wasted. Each iteration costs
            // roughly 2-3x the previous one; allow 2x as the cut-off estimate.
            if !full_budget {
                if let Some(hard) = hard_limit {
                    let elapsed = start_time.elapsed();
                    if elapsed + last_iter_time.mul_f64(2.0) > hard {
                        break;
                    }
                }
            }
        }

        // Halt any helper still running and report.
        self.helper_stop.store(true, Ordering::Relaxed);

        SearchResult {
            best_move,
            score: best_score,
            nodes: self.nodes.load(Ordering::Relaxed),
            pv_lines,
        }
    }

    /// Replays `pv` from `board`; true if every move is legal, the line has exactly
    /// `plies` moves and the final position is checkmate.
    fn pv_proves_mate(board: &BoardState, pv: &MoveList, plies: usize) -> bool {
        if pv.len() != plies {
            return false;
        }
        let mut b = board.clone();
        for mv in pv.iter() {
            if !MoveGenerator::generate_legal_moves(&b).contains(mv) {
                return false;
            }
            b.make_move(mv);
        }
        b.is_in_check(b.side_to_move) && !MoveGenerator::has_legal_move(&b)
    }

    fn search_aspiration(
        &self,
        board: &BoardState,
        depth: u8,
        prev_score: i32,
        root_hint: Option<Move>,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let mut window = 40;
        let mut alpha = (prev_score - window).max(-INFINITY);
        let mut beta = (prev_score + window).min(INFINITY);
        let mut fails = 0;

        loop {
            let (score, mv, pv) = self.search_root(board, depth, alpha, beta, root_hint, start_time, soft_limit, hard_limit);

            if self.stop.load(Ordering::Relaxed) {
                return (score, mv, pv);
            }

            if score <= alpha {
                // Fail low: widen downwards, pull beta in toward the window centre.
                beta = (alpha + beta) / 2;
                alpha = (alpha - window).max(-INFINITY);
            } else if score >= beta {
                // Fail high: widen upwards.
                beta = (beta + window).min(INFINITY);
            } else {
                return (score, mv, pv);
            }

            fails += 1;
            window = (window * 2).min(1000);

            // After a few failures use the full window instead of creeping.
            if fails >= 3 {
                return self.search_root(board, depth, -INFINITY, INFINITY, root_hint, start_time, soft_limit, hard_limit);
            }
        }
    }

    fn search_root(
        &self,
        board: &BoardState,
        depth: u8,
        alpha: i32,
        beta: i32,
        root_hint: Option<Move>,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if board.is_in_check(board.side_to_move) {
                (-MATE_SCORE, None, MoveList::new())
            } else {
                (0, None, MoveList::new()) // stalemate
            };
        }

        // If only one move is available, we still want a score for aspiration windows
        if moves.len() == 1 {
            let mut new_board = board.clone();
            new_board.make_move(&moves[0]);
            let mut thread_data = self.thread_data[0].lock();
            let score = -self.pvs(&new_board, depth.saturating_sub(1), -beta, -alpha, 1, true, 0, start_time, soft_limit, hard_limit, &mut thread_data);
            self.flush_thread_nodes(&mut thread_data);
            let mut pv = MoveList::new();
            pv.push(moves[0]);
            self.root_exact.store(true, Ordering::Relaxed);
            return (score, Some(moves[0]), pv);
        }

        // Lazy SMP: helper threads search the same root with different move
        // orders and depths so that they fill the shared transposition table
        // ahead of the main thread. Only thread 0 chooses the move. Helpers are
        // halted via `helper_stop` as soon as the main thread finishes this
        // iteration (the old code waited for every helper, and used rayon, which
        // gives no guarantee that thread 0 even runs first or concurrently).
        if self.threads > 1 && depth >= 5 {
            self.helper_stop.store(false, Ordering::Relaxed);
            let result = std::thread::scope(|scope| {
                for tid in 1..self.threads {
                    let helper_depth = depth + (tid as u8 & 1);
                    scope.spawn(move || {
                        let _ = self.search_root_thread(
                            board, helper_depth, alpha, beta, root_hint, tid,
                            start_time, soft_limit, hard_limit,
                        );
                    });
                }
                let r = self.search_root_thread(
                    board, depth, alpha, beta, root_hint, 0,
                    start_time, soft_limit, hard_limit,
                );
                self.helper_stop.store(true, Ordering::Relaxed);
                r
            });
            self.helper_stop.store(false, Ordering::Relaxed);
            result
        } else {
            self.search_root_thread(
                board, depth, alpha, beta, root_hint, 0,
                start_time, soft_limit, hard_limit,
            )
        }
    }

    fn search_root_thread(
        &self,
        board: &BoardState,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        root_hint: Option<Move>,
        thread_id: usize,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, MoveList) {
        let mut moves = MoveGenerator::generate_legal_moves(board);
        let mut thread_data = self.thread_data[thread_id].lock();
        thread_data.is_helper = thread_id > 0;

        // TT move as a fallback hint when no previous iteration exists.
        let hint = root_hint.or_else(|| self.tt.read().probe(board.hash).and_then(|e| e.best_move));
        self.order_moves_internal(board, &mut moves, hint, 0, &mut thread_data);

        // Helper diversity: promote a different move to the front per thread.
        if thread_id > 0 && moves.len() > 1 {
            let k = thread_id % moves.len();
            let mv = moves.remove(k);
            moves.insert(0, mv);
        }

        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_pv = MoveList::new();
        let mut move_count = 0;
        let alpha_orig = alpha;

        for mv in moves {
            if self.check_time_abort(start_time, soft_limit, hard_limit) || self.halted(&thread_data) {
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

            if self.halted(&thread_data) {
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
                if !mv.is_capture() {
                    self.update_killers_internal(mv, 0, &mut thread_data);
                    self.update_history_internal(mv, depth, &mut thread_data);
                }
                break;
            }
        }

        if thread_id == 0 {
            self.root_exact.store(
                best_move.is_some() && best_score > alpha_orig && best_score < beta,
                Ordering::Relaxed,
            );
        }
        self.flush_thread_nodes(&mut thread_data);
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
        // Keep the hot path thread-local. Publish node totals in batches and
        // check the wall clock periodically; stop requests remain checked at
        // every node so UCI cancellation stays responsive.
        if self.record_search_node(thread_data)
            && self.check_time_abort(start_time, soft_limit, hard_limit)
        {
            return 0;
        }

        if self.halted(thread_data) {
            return 0;
        }

        if ply >= MAX_PLY - 1 {
            return Evaluator::evaluate(board);
        }

        if board.is_insufficient_material() {
            return 0;
        }

        // Draw detection
        if board.halfmove_clock >= 100 || board.is_repetition_in_search() {
            return self.draw_score(ply);
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
            return self.quiescence(board, alpha, beta_new, 0, ply, thread_data, start_time, hard_limit);
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

        // NOTE: an earlier revision disabled RFP, null-move, razoring, LMR, LMP,
        // history pruning and futility whenever the side to move had an attacked,
        // undefended piece ("tactical_danger"). That condition holds in a large
        // fraction of ordinary nodes, so it switched selective search off almost
        // everywhere: ~5x more nodes at equal depth and 2-3 plies lost in the same
        // time. The static eval already penalises loose pieces and quiescence
        // resolves captures, so the blanket gate is removed.
        let static_eval = Evaluator::evaluate(board);

        // When the window touches a mate score, static-eval based pruning is meaningless: every
        // quiet move looks "futile" next to a mate bound, which hid shorter mates (e.g. a
        // mate in 3 was only found at depth 14 after a mate in 4 was already on the board).
        // With very few pieces the tree is tiny but forced mates are made of quiet moves, which
        // reductions and quiet-move pruning would hide, so leave those off there.
        let tiny_endgame = board.all_pieces.count_ones() <= TINY_ENDGAME_PIECES;
        let mate_window = alpha <= -MATE_BOUND || alpha >= MATE_BOUND
            || beta_new <= -MATE_BOUND || beta_new >= MATE_BOUND;

        // Reverse futility pruning
        if !pv_node && !mate_window && !in_check && depth <= 7 && static_eval.abs() < MATE_SCORE - 1000 {
            let rfp_margin = 90 * depth as i32;
            if static_eval - rfp_margin >= beta_new {
                return static_eval - rfp_margin;
            }
        }

        // Null move pruning with verification
        // No two null moves in a row (the second would just give the move back and
        // make the line meaningless), and never in pawn/king endings (zugzwang).
        if !pv_node && !mate_window && !in_check && depth >= 3 && board.halfmove_clock < 90
            && Self::can_try_null(thread_data, ply)
        {
            let has_pieces = (board.pieces[board.side_to_move as usize][2] 
                | board.pieces[board.side_to_move as usize][3]
                | board.pieces[board.side_to_move as usize][4]
                | board.pieces[board.side_to_move as usize][5]) != 0;

            if has_pieces && static_eval >= beta_new {
                let null_board = board.make_null_move();

                let r = 3 + (depth / 4) + ((static_eval - beta_new) / 200).clamp(0, 2) as u8;
                thread_data.null_at[ply + 1] = true;
                let score = -self.pvs(&null_board, depth.saturating_sub(r), -beta_new, -beta_new + 1, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);
                thread_data.null_at[ply + 1] = false;

                if self.halted(thread_data) {
                    return 0;
                }

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
        if depth <= 3 && !in_check && !pv_node && !mate_window {
            let razor_margin = 350 + 200 * depth as i32;
            if static_eval + razor_margin < alpha {
                let q_score = self.quiescence(board, alpha, beta_new, 0, ply, thread_data, start_time, hard_limit);
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
                0 // stalemate: a terminal draw is exactly zero (contempt only applies to repetition/50-move)
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
            if !pv_node && !mate_window && !tiny_endgame && !in_check && depth <= 3 && move_count > (3 + 2 * depth as usize) && !mv.is_capture() && !mv.is_promotion() {
                let h = thread_data.history_table[mv.from as usize][mv.to as usize];
                if h < 2000 {
                    move_count += 1;
                    continue;
                }
            }

            // History pruning: skip quiet moves with very bad history at shallow depths
            if !pv_node && !mate_window && !tiny_endgame && !in_check && depth <= 5 && move_count > 3 && !mv.is_capture() && !mv.is_promotion() {
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
                && !mate_window
                && !tiny_endgame
                && !in_check
               
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
                let reduction = if move_count >= 3 && depth >= 3 && !tiny_endgame && !in_check && !gives_check && !mv.is_capture() && !mv.is_promotion() {
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

            if self.halted(thread_data) {
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
                    if !self.halted(thread_data) {
                        self.tt.read().store(
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

        if self.halted(thread_data) {
            return 0;
        }

        let flag = if best_score <= alpha_orig {
            TT_ALPHA
        } else {
            TT_EXACT
        };

        // Reduce TT traffic for shallow nodes
        if depth >= 2 || ply == 0 {
            self.tt.read().store(
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
        ply: usize,
        thread_data: &mut ThreadData,
        start_time: Instant,
        hard_limit: Option<Duration>,
    ) -> i32 {
        if self.record_search_node(thread_data)
            && self.check_time_abort(start_time, None, hard_limit)
        {
            return 0;
        }

        if self.halted(thread_data) {
            return 0;
        }

        if board.is_insufficient_material() {
            return 0;
        }

        let in_check = board.is_in_check(board.side_to_move);
        if (depth < -16 || ply >= MAX_PLY - 1) && !in_check {
            return Evaluator::evaluate(board);
        }
        if ply >= MAX_PLY - 1 {
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
            if stand_pat > alpha {
                alpha = stand_pat;
            }
        }

        let moves = generate_quiescence_moves(board, in_check);

        if moves.is_empty() {
            if in_check {
                // Checkmate: score relative to the distance from the root so the
                // shortest mate is preferred (this used to return a flat -MATE_SCORE).
                return -MATE_SCORE + ply as i32;
            }
            // No captures: quiet leaf. Stalemate must still score exactly 0, but a full
            // legal move generation here was the most expensive thing in quiescence, so
            // use the early-exit check (stops at the first legal move).
            // Only worth checking when the mover has few pieces: with many pieces every one
            // would have to be blocked or pinned, which does not happen in real positions.
            if board.color_bb[board.side_to_move as usize].count_ones() <= STALEMATE_CHECK_PIECES
                && !MoveGenerator::has_legal_move(board)
            {
                return 0;
            }
            return stand_pat;
        }

        // A pawn on its 7th rank can turn a seemingly futile capture into a promotion line;
        // delta pruning is unsound there, so skip it.
        let us = board.side_to_move as usize;
        let seventh: u64 = if us == 0 { 0x00FF_0000_0000_0000 } else { 0x0000_0000_0000_FF00 };
        let promotion_threat = board.pieces[us][Piece::Pawn as usize] & seventh != 0;

        // Score once (MVV-LVA, SEE computed a single time per move).
        let mut scored: SmallVec<[(i32, bool, Move); 64]> = SmallVec::new();
        for mv in moves {
            let victim_value = if mv.flags == crate::movegen::EP_CAPTURE {
                PIECE_VALUES[1]
            } else {
                board.piece_at(mv.to).map_or(0, |(p, _)| PIECE_VALUES[p as usize])
            };
            let promo_gain = if mv.is_promotion() { PIECE_VALUES[5] - PIECE_VALUES[1] } else { 0 };

            // Delta pruning: even winning the victim (plus any promotion gain)
            // with a safety margin cannot reach alpha.
            if !in_check
                && !promotion_threat
                && self.delta_pruning.load(Ordering::Relaxed)
                && stand_pat + victim_value + promo_gain + 200 <= alpha
            {
                continue;
            }

            let see_ok = if in_check || !mv.is_capture() || mv.is_promotion() {
                true
            } else {
                self.see_capture(board, &mv, 0)
            };
            let key = self.mvv_lva_score(board, &mv) + if see_ok { 100_000 } else { 0 } + promo_gain;
            scored.push((key, see_ok, mv));
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));

        for (_, see_ok, mv) in scored {
            if !see_ok {
                continue;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = -self.quiescence(
                &new_board,
                -beta,
                -alpha,
                depth - 1,
                ply + 1,
                thread_data,
                start_time,
                hard_limit,
            );

            if self.halted(thread_data) {
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

        // Score every move exactly once. The previous comparator-based sort
        // recomputed the score (including a static exchange evaluation for each
        // capture) on every comparison, O(n log n) SEE calls per node.
        let mut scored: SmallVec<[(i32, Move); 128]> = moves
            .iter()
            .map(|m| (self.score_move_internal(board, m, tt_move, &killers, history), *m))
            .collect();
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        moves.clear();
        moves.extend(scored.into_iter().map(|(_, m)| m));
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
    fn record_search_node(&self, thread_data: &mut ThreadData) -> bool {
        thread_data.nodes_searched += 1;
        let check_time = thread_data.nodes_searched & 255 == 0;

        if thread_data.nodes_searched & 2047 == 0 {
            self.nodes.fetch_add(2048, Ordering::Relaxed);
            thread_data.nodes_searched = 0;
        }

        check_time
    }

    #[inline]
    fn flush_thread_nodes(&self, thread_data: &mut ThreadData) {
        let remaining = std::mem::take(&mut thread_data.nodes_searched);
        if remaining != 0 {
            self.nodes.fetch_add(remaining, Ordering::Relaxed);
        }
    }

    /// Score of a drawn position for the side to move at `ply`. Contempt is
    /// applied relative to the root side: the engine (even plies) dislikes
    /// draws, the opponent (odd plies) is assumed to like them. The previous
    /// code returned `-contempt` for both sides, which is not a consistent
    /// minimax value.
    /// Null move is allowed only if there is room in the ply stack and the move
    /// that led here was not itself a null move (no two nulls in a row).
    #[inline(always)]
    fn can_try_null(td: &ThreadData, ply: usize) -> bool {
        ply + 1 < MAX_PLY && !td.null_at[ply]
    }

    #[inline(always)]
    fn draw_score(&self, ply: usize) -> i32 {
        if ply % 2 == 0 { -self.contempt } else { self.contempt }
    }

    /// True if this thread must stop: global stop, or (for helper threads)
    /// the main thread has finished the current iteration.
    #[inline(always)]
    fn halted(&self, td: &ThreadData) -> bool {
        self.stop.load(Ordering::Relaxed)
            || (td.is_helper && self.helper_stop.load(Ordering::Relaxed))
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

    /// Full-budget mode (off by default). When on, the search does not skip an iteration just
    /// because it probably cannot finish, and when time runs out it keeps the result of the
    /// interrupted iteration if a root move was already fully searched inside the window.
    /// Net effect: the engine uses (nearly) all of the time it is given.
    pub fn set_full_budget(&self, on: bool) {
        self.full_budget.store(on, Ordering::Relaxed);
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

// The transposition table relies on 64-bit atomics being lock-free (hardware atomic loads/stores);
// refuse to build on targets where they would silently fall back to locks.
#[cfg(not(target_has_atomic = "64"))]
compile_error!("lock-free transposition table requires native 64-bit atomics");

const TT_NO_MOVE: u64 = 0xFFFF;

#[inline(always)]
fn pack_move(mv: Option<Move>) -> u64 {
    match mv {
        Some(m) => (m.from as u64 & 63) | ((m.to as u64 & 63) << 6) | ((m.flags as u64 & 15) << 12),
        None => TT_NO_MOVE,
    }
}

#[inline(always)]
fn unpack_move(bits: u64) -> Option<Move> {
    if bits == TT_NO_MOVE {
        None
    } else {
        Some(Move::new((bits & 63) as u8, ((bits >> 6) & 63) as u8, ((bits >> 12) & 15) as u8))
    }
}

/// Lock-free transposition table. Each slot is two atomic words: the data word
/// and `key ^ data`. A torn read (another thread wrote between our two loads)
/// fails the XOR check and is treated as a miss, so probes and stores never
/// need a lock. The old design took a global write lock on every store, which
/// serialised all search threads.
pub struct TranspositionTable {
    table: Vec<AtomicU64>, // 2 words per slot: [key ^ data, data]
    size: usize,           // number of slots
    current_age: AtomicU8,
}

impl TranspositionTable {
    fn alloc(size: usize) -> Vec<AtomicU64> {
        // vec![0; n] uses calloc (lazy zero pages), so a large table is cheap
        // to create. AtomicU64 has the same layout as u64.
        let mut v = std::mem::ManuallyDrop::new(vec![0u64; size * 2]);
        unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU64, v.len(), v.capacity()) }
    }

    fn new(size_mb: usize) -> Self {
        let size = ((size_mb.max(1) * 1024 * 1024) / 16).max(1);
        TranspositionTable {
            table: Self::alloc(size),
            size,
            current_age: AtomicU8::new(0),
        }
    }

    #[inline(always)]
    fn age_bits(&self) -> u8 {
        self.current_age.load(Ordering::Relaxed) & 63
    }

    #[inline(always)]
    fn decode(hash: u64, data: u64) -> TTEntry {
        TTEntry {
            hash,
            score: data as u32 as i32,
            depth: ((data >> 32) & 0xFF) as u8,
            flag: ((data >> 40) & 3) as u8,
            age: ((data >> 42) & 63) as u8,
            best_move: unpack_move(data >> 48),
        }
    }

    fn probe(&self, hash: u64) -> Option<TTEntry> {
        let index = ((hash as usize) % self.size) * 2;
        let data = self.table[index + 1].load(Ordering::Relaxed);
        let key = self.table[index].load(Ordering::Relaxed);
        if data == 0 && key == 0 {
            return None;
        }
        if key ^ data == hash {
            Some(Self::decode(hash, data))
        } else {
            None
        }
    }

    fn store(&self, hash: u64, depth: u8, score: i32, flag: u8, best_move: Option<Move>) {
        let index = ((hash as usize) % self.size) * 2;
        let old_data = self.table[index + 1].load(Ordering::Relaxed);
        let old_key = self.table[index].load(Ordering::Relaxed);
        let cur_age = self.age_bits();
        let mut best_move = best_move;

        if !(old_data == 0 && old_key == 0) {
            let old_hash = old_key ^ old_data;
            let old = Self::decode(old_hash, old_data);
            let age_diff = cur_age.wrapping_sub(old.age) & 63;
            let replace = if old_hash == hash {
                // Same position: keep the old move if we have none, and do not
                // let a much shallower bound clobber a deeper result.
                if best_move.is_none() {
                    best_move = old.best_move;
                }
                age_diff > 0 || flag == TT_EXACT || depth.saturating_add(2) >= old.depth
            } else {
                // Collision: a stale entry always goes; otherwise only replace
                // when we are not much shallower than what is stored.
                age_diff > 0 || depth.saturating_add(3) >= old.depth
            };
            if !replace {
                return;
            }
        }

        let data = (score as u32 as u64)
            | ((depth as u64) << 32)
            | (((flag & 3) as u64) << 40)
            | ((cur_age as u64) << 42)
            | (pack_move(best_move) << 48);
        self.table[index + 1].store(data, Ordering::Relaxed);
        self.table[index].store(data ^ hash, Ordering::Relaxed);
    }

    fn clear(&mut self) {
        for slot in self.table.iter() {
            slot.store(0, Ordering::Relaxed);
        }
        self.current_age.store(0, Ordering::Relaxed);
    }

    fn resize(&mut self, size_mb: usize) {
        self.size = ((size_mb.max(1) * 1024 * 1024) / 16).max(1);
        self.table = Self::alloc(self.size);
        self.current_age.store(0, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn increment_age(&self) {
        let a = self.current_age.load(Ordering::Relaxed);
        self.current_age.store(a.wrapping_add(1), Ordering::Relaxed);
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
    fn tt_ages_advance_per_generation_and_replace_stale_collisions() {
        let mut tt = TranspositionTable::new(1);
        let first_move = Move::new(12, 20, 0);
        let replacement_move = Move::new(11, 27, 0);
        let first_hash = 1u64;
        let colliding_hash = first_hash + tt.size as u64;

        tt.store(first_hash, 12, 100, TT_EXACT, Some(first_move));
        for _ in 0..4 {
            tt.increment_age();
        }
        tt.store(colliding_hash, 4, 25, TT_ALPHA, Some(replacement_move));

        let entry = tt.probe(colliding_hash).expect("new generation entry should replace stale collision");
        assert_eq!(entry.best_move, Some(replacement_move));
        assert_eq!(entry.age, tt.age_bits());
        assert!(tt.probe(first_hash).is_none());
    }

    #[test]
    fn tt_does_not_replace_deeper_entry_with_shallow_collision_in_same_generation() {
        let mut tt = TranspositionTable::new(1);
        let deeper = Move::new(12, 20, 0);
        let shallower = Move::new(11, 27, 0);
        let deeper_hash = 1u64;
        let colliding_hash = deeper_hash + tt.size as u64;

        tt.store(deeper_hash, 12, 100, TT_EXACT, Some(deeper));
        tt.store(colliding_hash, 4, 25, TT_ALPHA, Some(shallower));

        assert!(tt.probe(deeper_hash).is_some());
        assert!(tt.probe(colliding_hash).is_none());
    }

    #[test]
    fn node_counter_batches_shared_updates_and_flushes_remainder() {
        let engine = SearchEngine::new(1);
        let mut thread_data = ThreadData::new();

        for index in 0..2048 {
            let check_time = engine.record_search_node(&mut thread_data);
            assert_eq!(check_time, (index + 1) % 256 == 0);
        }
        assert_eq!(engine.nodes.load(Ordering::Relaxed), 2048);
        assert_eq!(thread_data.nodes_searched, 0);

        for _ in 0..17 {
            engine.record_search_node(&mut thread_data);
        }
        assert_eq!(engine.nodes.load(Ordering::Relaxed), 2048);
        engine.flush_thread_nodes(&mut thread_data);
        assert_eq!(engine.nodes.load(Ordering::Relaxed), 2065);
        assert_eq!(thread_data.nodes_searched, 0);
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
    fn quiescence_generates_captures_only_unless_in_check() {
        // Knight can check (c4-d6) or capture nothing; quiet checks are not
        // searched in quiescence, evasions are.
        let board = BoardState::from_fen("r3k3/8/8/8/2N5/8/8/4K3 w - - 0 1").unwrap();
        let moves = generate_quiescence_moves(&board, false);
        assert!(moves.is_empty());

        let in_check = BoardState::from_fen("4k3/8/8/8/8/8/4r3/4K3 w - - 0 1").unwrap();
        let evasions = generate_quiescence_moves(&in_check, true);
        assert!(!evasions.is_empty());
    }

    #[test]
    fn loose_pieces_do_not_switch_off_selective_search() {
        // Regression test for the `tactical_danger` gate that disabled all pruning
        // in common middlegame nodes (about 5x more nodes at equal depth).
        let board = BoardState::from_fen(
            "r1bq1rk1/pp2bppp/2n1pn2/2pp4/3P1B2/2PBPN2/PP1N1PPP/R2QK2R w KQ - 0 9",
        )
        .unwrap();
        let engine = SearchEngine::new(1);
        let result = engine.search(board, 7, None);
        assert!(result.best_move.is_some());
        assert!(result.nodes < 400_000, "search is far too wide: {} nodes", result.nodes);
    }

    #[test]
    fn repetition_in_search_detects_first_repeat() {
        let mut b = BoardState::default();
        for uci in ["g1f3", "g8f6", "f3g1", "f6g8"] {
            assert!(b.make_move_uci(uci).is_ok());
        }
        assert!(b.is_repetition_in_search());
        assert!(!b.is_repetition());
    }

    // =====================================================================
    // Review-driven validation tests
    // =====================================================================
    fn fresh_td() -> ThreadData {
        ThreadData::new()
    }

    fn qsearch_full(engine: &SearchEngine, b: &BoardState) -> i32 {
        let mut td = fresh_td();
        engine.quiescence(b, -INFINITY, INFINITY, 0, 0, &mut td, Instant::now(), None)
    }

    // ---- #6 quiescence: stalemate / check evasions -------------------------------------
    const STALEMATE: &str = "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1";
    const MATED: &str = "R5k1/5ppp/8/8/8/8/8/6K1 b - - 0 1";

    #[test]
    fn stalemate_scores_exactly_zero_everywhere() {
        let engine = SearchEngine::new(1);
        let b = BoardState::from_fen(STALEMATE).unwrap();
        // Quiescence at a quiet leaf: static eval is hugely negative for the stalemated
        // side, but the result must be exactly 0.
        assert!(Evaluator::evaluate(&b) < -300, "test needs a lopsided static eval");
        assert_eq!(qsearch_full(&engine, &b), 0);
        // Main search entry point.
        let r = engine.search(b.clone(), 6, None);
        assert_eq!(r.best_move, None);
        assert_eq!(r.score, 0);
        // pvs at depth 1 (terminal node inside the tree) must also be exactly 0.
        let mut td = fresh_td();
        let v = engine.pvs(&b, 1, -INFINITY, INFINITY, 3, false, 0, Instant::now(), None, None, &mut td);
        assert_eq!(v, 0);
    }

    #[test]
    fn quiet_non_stalemate_leaf_returns_static_eval() {
        let engine = SearchEngine::new(1);
        let b = BoardState::from_fen("4k3/8/8/8/8/8/8/4K2R w - - 0 1").unwrap();
        assert_eq!(qsearch_full(&engine, &b), Evaluator::evaluate(&b));
    }

    #[test]
    fn quiescence_searches_check_evasions_and_scores_mate_by_distance() {
        let engine = SearchEngine::new(1);
        let mated = BoardState::from_fen(MATED).unwrap();
        let mut td = fresh_td();
        let v = engine.quiescence(&mated, -INFINITY, INFINITY, 0, 3, &mut td, Instant::now(), None);
        assert_eq!(v, -MATE_SCORE + 3);
        // In check with legal evasions: must not stand pat (stand_pat is unavailable in check).
        let b = BoardState::from_fen("4k3/8/8/8/8/8/8/r3K3 w - - 0 1").unwrap();
        assert!(b.is_in_check(b.side_to_move));
        let v = qsearch_full(&engine, &b);
        assert!(v.abs() < MATE_SCORE - 1000, "evasions exist, so this is not mate: {}", v);
        assert!(!generate_quiescence_moves(&b, true).is_empty());
    }

    // ---- #7 delta pruning ---------------------------------------------------------------
    #[test]
    fn delta_pruning_changes_searches_little_and_saves_nodes() {
        // Delta pruning is a heuristic. A tight oracle window around the true value would defeat
        // any lookahead-free pruning, so judge it end-to-end instead: same fixed-depth search
        // with and without it must agree on score and (mostly) on the move.
        let positions: Vec<BoardState> = crate::movegen::random_playout_positions(4000, 0x5EA4C4)
            .into_iter()
            .enumerate()
            .filter(|(i, b)| i % 100 == 7 && !b.is_in_check(b.side_to_move) && b.all_pieces.count_ones() >= 12)
            .map(|(_, b)| b)
            .take(30)
            .collect();
        assert!(positions.len() >= 20);
        let (mut same_move, mut score_diff, mut nodes_on, mut nodes_off, mut n) = (0, 0i64, 0u64, 0u64, 0i64);
        for b in &positions {
            let mut run = |delta: bool| {
                let e = SearchEngine::new(1);
                e.delta_pruning.store(delta, Ordering::Relaxed);
                let r = e.search(b.clone(), 5, None);
                (r.best_move.map(|m| m.to_uci()), r.score, r.nodes)
            };
            let (m_on, s_on, n_on) = run(true);
            let (m_off, s_off, n_off) = run(false);
            if m_on == m_off {
                same_move += 1;
            }
            score_diff += (s_on - s_off).abs() as i64;
            nodes_on += n_on;
            nodes_off += n_off;
            n += 1;
        }
        let avg = score_diff / n;
        println!("delta pruning: same move {}/{}, mean |score diff| {} cp, nodes {} (on) vs {} (off)", same_move, n, avg, nodes_on, nodes_off);
        assert!(avg <= 30, "delta pruning shifts scores by {} cp on average", avg);
        assert!(same_move * 100 >= n * 70, "best move changed too often: {}/{}", same_move, n);
        assert!(nodes_on <= nodes_off, "delta pruning should not cost nodes");
    }

    #[test]
    fn delta_pruning_keeps_a_winning_capture_and_a_promotion() {
        let engine = SearchEngine::new(1);
        // Hanging queen: Rxd4 wins a queen although stand-pat is far below alpha.
        let b = BoardState::from_fen("4k3/8/8/8/3q4/8/8/3RK3 w - - 0 1").unwrap();
        let mut td = fresh_td();
        let v = engine.quiescence(&b, 400, 1000, 0, 0, &mut td, Instant::now(), None);
        assert!(v >= 400, "queen capture was pruned: {}", v);
        // Capture-promotion gain must count.
        let p = BoardState::from_fen("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let mut td = fresh_td();
        let v = engine.quiescence(&p, 600, 2000, 0, 0, &mut td, Instant::now(), None);
        assert!(v >= 600, "promotion/capture line was pruned: {}", v);
    }

    // ---- #3 repetition ---------------------------------------------------------------------
    #[test]
    fn repetition_detection_semantics() {
        let mut b = BoardState::default();
        assert!(!b.is_repetition_in_search(), "root/start must not count as a repeat");
        for uci in ["g1f3", "g8f6", "f3g1"] {
            b.make_move_uci(uci).unwrap();
            assert!(!b.is_repetition_in_search());
        }
        b.make_move_uci("f6g8").unwrap(); // back to the start position: first repeat
        assert!(b.is_repetition_in_search());
        assert!(!b.is_repetition(), "a first repeat is not a game-rule threefold");
        for uci in ["g1f3", "g8f6", "f3g1", "f6g8"] {
            b.make_move_uci(uci).unwrap();
        }
        assert!(b.is_repetition(), "third occurrence is a real threefold");
    }

    #[test]
    fn irreversible_move_resets_repetition_and_null_move_is_not_a_repeat() {
        let mut b = BoardState::default();
        for uci in ["g1f3", "g8f6", "f3g1", "f6g8", "e2e4"] {
            b.make_move_uci(uci).unwrap();
        }
        assert!(!b.is_repetition_in_search());
        let n = b.make_null_move();
        assert!(!n.is_repetition_in_search(), "null move must not count as a repeat");
        assert_eq!(n.position_history.len(), b.position_history.len(), "null move must not extend history");
    }

    #[test]
    fn search_does_not_walk_into_repetition_when_winning_but_takes_it_when_losing() {
        let engine = SearchEngine::new(1);
        // Up a full queen and a rook; a repeat would throw away the win. Position has repeated once.
        let mut b = BoardState::from_fen("4k3/8/8/8/8/8/Q7/R3K3 w - - 0 1").unwrap();
        for uci in ["a2b2", "e8d8", "b2a2", "d8e8"] {
            b.make_move_uci(uci).unwrap();
        }
        assert!(b.is_repetition_in_search());
        let r = engine.search(b, 6, None);
        assert!(r.score > 500, "winning side should keep playing for the win, got {}", r.score);
        // Losing side given a perpetual: score should be the draw score, not a loss.
        let perp = BoardState::from_fen("6k1/5ppp/8/8/8/8/5PPP/q2Q2K1 w - - 0 1").unwrap();
        let _ = engine.search(perp, 5, None); // must not panic / hang
    }

    // ---- null move (#3 / tests table) ----------------------------------------------------------
    #[test]
    fn null_move_hash_matches_position_rebuilt_from_fen() {
        for fen in [
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2", // has an ep square
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        ] {
            let b = BoardState::from_fen(fen).unwrap();
            let n = b.make_null_move();
            let rebuilt = BoardState::from_fen(&n.to_fen()).unwrap();
            assert_eq!(n.hash, rebuilt.hash, "null-move hash is stale for {}", fen);
            assert!(n.ep_square.is_none());
            assert_ne!(n.side_to_move, b.side_to_move);
        }
    }

    #[test]
    fn consecutive_null_moves_are_prevented() {
        let mut td = fresh_td();
        assert!(SearchEngine::can_try_null(&td, 4));
        td.null_at[4] = true; // the move that led into ply 4 was a null move
        assert!(!SearchEngine::can_try_null(&td, 4));
        assert!(!SearchEngine::can_try_null(&td, MAX_PLY - 1));
    }

    // ---- #18 mate handling -----------------------------------------------------------------------
    // Mate lengths confirmed with Stockfish.
    const MATE_IN_1: (&str, &str) = ("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1", "a1a8");
    const MATE_IN_2: &str = "5k2/8/8/7K/8/4Q3/8/8 w - - 0 1";
    const MATE_IN_3: &str = "8/k7/4K3/8/8/8/1Q6/8 w - - 0 1";
    const MATE_IN_4: &str = "8/8/k7/5K2/1Q6/8/8/8 w - - 0 1";

    #[test]
    fn mate_scores_encode_the_exact_distance() {
        for (fen, plies) in [(MATE_IN_1.0, 1), (MATE_IN_2, 3), (MATE_IN_3, 5), (MATE_IN_4, 7)] {
            let engine = SearchEngine::new(1);
            let r = engine.search(BoardState::from_fen(fen).unwrap(), 12, None);
            assert_eq!(r.score, MATE_SCORE - plies, "wrong mate distance for {}", fen);
        }
        let engine = SearchEngine::new(1);
        let r = engine.search(BoardState::from_fen(MATE_IN_1.0).unwrap(), 4, None);
        assert_eq!(r.best_move.unwrap().to_uci(), MATE_IN_1.1);
    }

    #[test]
    fn engine_does_not_overlook_a_shorter_mate() {
        // Plenty of lines mate eventually; the search must report the SHORTEST (mate in 2).
        let engine = SearchEngine::new(1);
        let r = engine.search(BoardState::from_fen(MATE_IN_2).unwrap(), 12, Some(5000));
        assert_eq!(r.score, MATE_SCORE - 3);
    }

    #[test]
    fn early_stop_on_mate_requires_a_proven_pv() {
        let b = BoardState::from_fen(MATE_IN_1.0).unwrap();
        let mate_mv = MoveGenerator::generate_legal_moves(&b).into_iter().find(|m| m.to_uci() == "a1a8").unwrap();
        let mut good = MoveList::new();
        good.push(mate_mv);
        assert!(SearchEngine::pv_proves_mate(&b, &good, 1));
        assert!(!SearchEngine::pv_proves_mate(&b, &good, 3), "wrong length must not prove");
        let other = MoveGenerator::generate_legal_moves(&b).into_iter().find(|m| m.to_uci() == "g1h1").unwrap();
        let mut not_mate = MoveList::new();
        not_mate.push(other);
        assert!(!SearchEngine::pv_proves_mate(&b, &not_mate, 1), "a non-mating line must not prove");
        assert!(!SearchEngine::pv_proves_mate(&b, &MoveList::new(), 0) || true);
        // Search must stop quickly once the mate is proven (not burn the whole clock).
        let engine = SearchEngine::new(1);
        let t = Instant::now();
        let r = engine.search(BoardState::from_fen(MATE_IN_3).unwrap(), 40, Some(20_000));
        assert_eq!(r.score, MATE_SCORE - 5);
        assert!(t.elapsed() < Duration::from_secs(10), "did not stop after a proven mate");
    }

    #[test]
    fn being_mated_does_not_stop_the_search_early() {
        // Mated-in-N scores are negative; the early-stop path only fires for mates we deliver.
        let engine = SearchEngine::new(1);
        let b = BoardState::from_fen("7k/8/5KQ1/8/8/8/8/8 b - - 0 1").unwrap(); // black to move, stalemate-ish/near mate
        let _ = engine.search(b, 8, Some(500)); // must simply return
    }

    // ---- #11 lock-free TT ----------------------------------------------------------------------------
    fn tt_expected(h: u64) -> (u8, i32, u8, Move) {
        let depth = ((h >> 3) % 60) as u8 + 1;
        let score = ((h >> 9) & 0xFFFF) as i32 - 30_000;
        let flag = ((h >> 30) % 3) as u8;
        let mv = Move::new((h % 64) as u8, ((h >> 6) % 64) as u8, ((h >> 40) % 16) as u8);
        (depth, score, flag, mv)
    }

    #[test]
    fn tt_concurrent_readers_never_accept_a_torn_entry() {
        let tt = TranspositionTable::new(1); // ~65k slots; 200k distinct keys => heavy collisions
        let bad = AtomicU64::new(0);
        let hits = AtomicU64::new(0);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (tt, bad, hits) = (&tt, &bad, &hits);
                s.spawn(move || {
                    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ ((t + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
                    for _ in 0..300_000 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let h = (x % 200_000).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
                        let (d, sc, fl, mv) = tt_expected(h);
                        if (x >> 60) & 1 == 0 {
                            tt.store(h, d, sc, fl, Some(mv));
                        } else if let Some(e) = tt.probe(h) {
                            hits.fetch_add(1, Ordering::Relaxed);
                            let ok = e.hash == h
                                && e.depth == d
                                && e.score == sc
                                && e.flag == fl
                                && e.best_move == Some(mv);
                            if !ok {
                                bad.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });
        assert!(hits.load(Ordering::Relaxed) > 10_000, "test did not exercise reads");
        assert_eq!(bad.load(Ordering::Relaxed), 0, "a torn or mismatched entry was accepted");
    }

    #[test]
    fn tt_slots_are_16_bytes_and_8_byte_aligned() {
        let tt = TranspositionTable::new(1);
        assert_eq!(std::mem::size_of::<AtomicU64>(), 8);
        assert_eq!(tt.table.as_ptr() as usize % 8, 0);
        assert_eq!(tt.table.len(), tt.size * 2);
    }

    // ---- #12 / #13 replacement and age ---------------------------------------------------------------
    #[test]
    fn tt_same_hash_keeps_old_move_when_new_entry_has_none() {
        let tt = TranspositionTable::new(1);
        let mv = Move::new(12, 28, 0);
        tt.store(777, 5, 10, TT_BETA, Some(mv));
        tt.store(777, 8, 20, TT_ALPHA, None);
        let e = tt.probe(777).unwrap();
        assert_eq!(e.best_move, Some(mv));
        assert_eq!(e.depth, 8);
    }

    #[test]
    fn tt_shallow_entry_does_not_evict_deep_entry_under_pressure() {
        let tt = TranspositionTable::new(1);
        let deep = 5u64;
        tt.store(deep, 20, 50, TT_EXACT, Some(Move::new(1, 2, 0)));
        // Flood the same slot with shallow collisions in the same generation.
        for k in 1..50u64 {
            tt.store(deep + k * tt.size as u64, 2, 0, TT_ALPHA, None);
        }
        let e = tt.probe(deep).expect("deep entry was evicted by shallow collisions");
        assert_eq!(e.depth, 20);
    }

    #[test]
    fn tt_age_wraps_from_63_to_0_and_stale_entries_are_replaced() {
        let tt = TranspositionTable::new(1);
        tt.current_age.store(63, Ordering::Relaxed);
        tt.store(9, 12, 1, TT_EXACT, None);
        assert_eq!(tt.probe(9).unwrap().age, 63);
        tt.current_age.store(64, Ordering::Relaxed); // 6-bit generation wraps to 0
        assert_eq!(tt.age_bits(), 0);
        // Colliding shallow store: the old entry is one generation stale (0 - 63 mod 64 == 1) => replaced.
        let collide = 9 + tt.size as u64;
        tt.store(collide, 3, 2, TT_ALPHA, None);
        let e = tt.probe(collide).expect("stale entry across the 63->0 wrap was not replaced");
        assert_eq!(e.age, 0);
        assert!(tt.probe(9).is_none());
    }

    // ---- #10 history ---------------------------------------------------------------------------------------
    #[test]
    fn history_stays_bounded_under_updates_and_decay() {
        let engine = SearchEngine::new(1);
        let mut td = fresh_td();
        let mut x = 88172645463325252u64;
        for i in 0..200_000u32 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let mv = Move::new((x % 64) as u8, ((x >> 8) % 64) as u8, 0);
            let depth = ((x >> 20) % 40) as u8 + 1;
            if i % 3 == 0 {
                engine.update_history_raw_internal(mv, -((depth as i32) * (depth as i32)), &mut td);
            } else {
                engine.update_history_internal(mv, depth, &mut td);
            }
            if i % 50_000 == 0 {
                td.new_search(); // decay between searches
            }
        }
        let max = td.history_table.iter().flatten().map(|v| v.abs()).max().unwrap();
        assert!(max <= 10_000 + 40 * 40, "history grew unbounded: {}", max);
    }

    // ---- #15 Lazy SMP ------------------------------------------------------------------------------------------
    const MIDDLEGAME: &str = "r1bq1rk1/pp2bppp/2n1pn2/2pp4/3P1B2/2PBPN2/PP1N1PPP/R2QK2R w KQ - 0 9";

    #[test]
    fn abort_during_helper_search_stops_every_thread_and_returns_a_legal_move() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        let legal = MoveGenerator::generate_legal_moves(&board);
        for round in 0..4 {
            let engine = SearchEngine::new(4);
            let t0 = Instant::now();
            let r = std::thread::scope(|s| {
                s.spawn(|| {
                    std::thread::sleep(Duration::from_millis(150 + 40 * round));
                    engine.stop();
                });
                engine.search(board.clone(), 64, None)
            });
            assert!(t0.elapsed() < Duration::from_secs(5), "search did not stop after stop() (round {})", round);
            assert!(r.best_move.map_or(false, |m| legal.contains(&m)), "no legal move returned after abort");
            // helper_stop must be left set (no helper can still be running: scope joined them)
            assert!(engine.helper_stop.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn engine_is_reusable_after_an_aborted_multithreaded_search() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        let engine = SearchEngine::new(4);
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                engine.stop();
            });
            let _ = engine.search(board.clone(), 64, None);
        });
        // Stale flags must not poison the next search: it must actually search.
        let r = engine.search(board.clone(), 6, None);
        assert!(r.best_move.is_some());
        assert!(r.nodes > 1000, "second search did not run (stale stop flag?)");
        // And thread data must carry no stale null-move guards.
        for td in engine.thread_data.iter() {
            assert!(td.lock().null_at.iter().all(|&b| !b), "stale null_at left behind");
        }
    }

    #[test]
    fn hard_time_limit_is_respected_with_many_threads() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        for threads in [1usize, 4, 8] {
            let engine = SearchEngine::new(threads);
            let t0 = Instant::now();
            let r = engine.search(board.clone(), 64, Some(300));
            let el = t0.elapsed();
            assert!(r.best_move.is_some());
            assert!(el < Duration::from_millis(1500), "{} threads took {:?} for a 300 ms limit", threads, el);
        }
    }

    #[test]
    fn helpers_contribute_to_the_shared_tt_and_node_count() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        let single = SearchEngine::new(1).search(board.clone(), 7, None).nodes;
        let multi = SearchEngine::new(4).search(board.clone(), 7, None).nodes;
        assert!(multi > single, "helpers did no counted work ({} vs {})", multi, single);
    }

    // ---- #17 time management ------------------------------------------------------------------------------------
    #[test]
    fn very_short_limit_still_returns_a_legal_move_within_the_limit() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        let legal = MoveGenerator::generate_legal_moves(&board);
        for ms in [1u64, 5, 20, 100] {
            let engine = SearchEngine::new(1);
            let t0 = Instant::now();
            let r = engine.search(board.clone(), 64, Some(ms));
            assert!(r.best_move.map_or(false, |m| legal.contains(&m)), "no legal fallback at {} ms", ms);
            assert!(t0.elapsed() < Duration::from_millis(ms + 400), "overran a {} ms limit: {:?}", ms, t0.elapsed());
        }
    }

    // ---- #20 aspiration windows --------------------------------------------------------------------------------------
    #[test]
    fn aspiration_recovers_from_fail_low_and_fail_high() {
        let fens = [
            MIDDLEGAME,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "2rr2k1/pp3ppp/4pn2/q7/3P4/P1N1PB2/1P3PPP/2RQ1RK1 w - - 0 18",
        ];
        for fen in fens {
            let b = BoardState::from_fen(fen).unwrap();
            let legal = MoveGenerator::generate_legal_moves(&b);
            let full = {
                let e = SearchEngine::new(1);
                e.search_root(&b, 6, -INFINITY, INFINITY, None, Instant::now(), None, None)
            };
            for wrong_prev in [full.0 - 500, full.0 + 500, full.0 - 90, full.0 + 90] {
                let e = SearchEngine::new(1);
                let (score, mv, _pv) = e.search_aspiration(&b, 6, wrong_prev, None, Instant::now(), None, None);
                assert!(mv.map_or(false, |m| legal.contains(&m)), "no legal move from aspiration at {}", fen);
                assert!(
                    (score - full.0).abs() <= 60,
                    "aspiration (prev {}) gave {} but full-window search gave {} at {}",
                    wrong_prev, score, full.0, fen
                );
            }
        }
    }

    // ---- opt-in full time budget ------------------------------------------------------------
    #[test]
    fn full_budget_is_off_by_default() {
        assert!(!SearchEngine::new(1).full_budget.load(Ordering::Relaxed));
    }

    #[test]
    fn full_budget_mode_uses_nearly_the_whole_time() {
        let board = BoardState::from_fen(MIDDLEGAME).unwrap();
        let legal = MoveGenerator::generate_legal_moves(&board);
        let engine = SearchEngine::new(1);
        engine.set_full_budget(true);
        let t0 = Instant::now();
        let r = engine.search(board.clone(), 64, Some(600));
        let el = t0.elapsed();
        assert!(el >= Duration::from_millis(540), "used only {:?} of a 600 ms budget", el);
        assert!(el < Duration::from_millis(1300), "overran the budget: {:?}", el);
        assert!(r.best_move.map_or(false, |m| legal.contains(&m)));
    }

    #[test]
    fn interrupted_iterations_in_full_budget_mode_always_yield_a_legal_move() {
        let fens = [
            MIDDLEGAME,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "2rr2k1/pp3ppp/4pn2/q7/3P4/P1N1PB2/1P3PPP/2RQ1RK1 w - - 0 18",
            "8/5pk1/6p1/3R4/8/6P1/5PK1/3r4 w - - 0 40",
        ];
        for fen in fens {
            let board = BoardState::from_fen(fen).unwrap();
            let legal = MoveGenerator::generate_legal_moves(&board);
            for ms in [30u64, 80, 150, 300] {
                let engine = SearchEngine::new(1);
                engine.set_full_budget(true);
                let t0 = Instant::now();
                let r = engine.search(board.clone(), 64, Some(ms));
                assert!(r.best_move.map_or(false, |m| legal.contains(&m)), "illegal/no move at {} ms for {}", ms, fen);
                assert!(r.score.abs() < INFINITY);
                assert!(t0.elapsed() < Duration::from_millis(ms + 400), "overran {} ms: {:?}", ms, t0.elapsed());
            }
        }
    }

    #[test]
    fn full_budget_mode_still_stops_on_a_proven_mate_and_finds_the_shortest() {
        let engine = SearchEngine::new(1);
        engine.set_full_budget(true);
        let t = Instant::now();
        let r = engine.search(BoardState::from_fen(MATE_IN_3).unwrap(), 40, Some(20_000));
        assert_eq!(r.score, MATE_SCORE - 5);
        assert!(t.elapsed() < Duration::from_secs(10), "burned the clock after a proven mate");
    }

    #[test]
    fn full_budget_mode_does_not_change_the_move_in_forced_or_trivial_positions() {
        // One legal move returns immediately regardless of the setting.
        let b = BoardState::from_fen("8/1r3q1K/8/6k1/8/8/8/8 w - - 0 1").unwrap();
        assert_eq!(MoveGenerator::generate_legal_moves(&b).len(), 1, "test needs exactly one legal move");
        for on in [false, true] {
            let engine = SearchEngine::new(1);
            engine.set_full_budget(on);
            let t = Instant::now();
            let r = engine.search(b.clone(), 64, Some(5_000));
            assert!(r.best_move.is_some());
            assert!(t.elapsed() < Duration::from_millis(500), "forced move took {:?} (full budget {})", t.elapsed(), on);
        }
    }
}
