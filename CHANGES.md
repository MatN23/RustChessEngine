# Strength regression fix

## Root cause
Commits 65807c5 / 29e527b / bea0e2e (Nov 2) added `tactical_danger`: whenever the side to
move had an attacked, undefended piece, reverse futility, null-move, razoring, LMR, LMP,
history pruning and futility were all switched off. That is true in a large share of
ordinary nodes, so selective search was disabled almost everywhere:
depth-7 cost 5.4M nodes (vs ~0.4M before) and quiescence (full legal movegen + board
copies at every leaf) made it worse. Under the Lichess bot's 0.5-5 s budget the engine
searched several plies shallower, and the global TT write lock hurt multi-thread runs.

## Changes
search.rs
- Removed the `tactical_danger` gating (static eval already scores loose pieces).
- Quiescence: captures only (+ evasions in check), no quiet-check generation, no
  stalemate movegen at quiet leaves, proper delta pruning, SEE computed once, mate score
  uses ply distance.
- Move ordering scores each move once (was recomputed inside the sort comparator).
- Lock-free transposition table (XOR-verified atomic slots); sane replacement policy
  (old one almost never replaced entries within a search).
- Repetition: first repeat in search is a draw (was 3-fold). Contempt now relative to the
  root side. Null move: no two in a row; en-passant key removed from the hash.
- Root: previous best move searched first; helpers use scoped threads and stop when the
  main thread finishes an iteration (rayon gave no guarantee thread 0 ran concurrently);
  forced move returns immediately; no iteration is started that cannot finish;
  history is decayed between searches instead of wiped.
eval.rs: attack maps built once per evaluation instead of four times.
movegen.rs / board.rs: legality test only for moves that can expose the king (king moves,
  en passant, pieces on a line with the king, or in check) using a history-free board copy.
Tests: updated TT/quiescence tests; added perft, captures-subset, repetition and a
  node-budget regression test (guards against pruning being disabled again).

## Verification (single thread, 1 core sandbox)
- perft exact on 10 positions; `cargo test`: all pass.
- Depth-7 benchmark over 6 positions: HEAD 5.4M nodes / 20.3 s  ->  0.43M nodes / 1.07 s.
- Head-to-head, 0.5 s/move, 12 games: fixed build 9 wins, 1 draw, 2 losses vs HEAD.

## Midgame evaluation fix (second part)
Observed: strong in the opening (book) and endgame, loses level-7 games in the middlegame.
Replaying lost games with Stockfish showed the engine's static eval was 100-240 cp
optimistic for ~40 plies of the middlegame while Stockfish had it 50-350 cp worse
(mean +163 cp bias; material+PST alone was closer to Stockfish than the full eval).
A least-squares fit of one weight per eval term on ~2,300 Stockfish-labelled midgame
positions (held-out check: 290 -> 177 cp mean error) found pawn_structure, king_safety,
space, rook and knight anti-correlated with Stockfish. The eval is colour-symmetric
(new test), so this is miscalibration, not a sign bug.
Fix: `EVAL_WEIGHTS` in eval.rs scales the terms; pawn structure, king safety, space, rook,
knight and tempo are halved (50%). Head-to-head vs the previous weights, same engine:
- 50% weights: 75% (10W 4D 2L, 300 ms/move) and 71.9% (9W 5D 2L, 500 ms/move)
- 25%: 62.5%, all five terms removed: 43.8%, regression-fit weights: 56%.
These are self-play results (32 games for the chosen setting). Against Stockfish level 7
at 10 s/move the engine was 0-2 in the only finished games BEFORE this change; it has not
been re-measured against Stockfish levels since this change.

## Review-driven fixes and validation (third part)
Real bugs found by writing tests for the review checklist:
- Missed shorter mates (pre-existing): a mate in 3 was reported as mate in 4/5/7 and only
  found at depth 14. Reverse futility and null-move pruning treated the defender as safe
  whenever the window was a mate score; razoring/futility/LMP/history pruning and LMR hid
  quiet mating moves. All are now off in mate windows, and quiet-move reductions/pruning are
  off at <= 4 pieces (TINY_ENDGAME_PIECES). Mate in 3 is now found at depth 5 and stable.
- The early stop on "mate found" now requires the PV to replay legally to checkmate in exactly
  the claimed number of plies, only for mates we deliver, and only once depth >= mate length.
- Stalemate now scores exactly 0 (it returned -contempt in the main search and was not detected
  in quiescence). Quiescence uses a cheap early-exit `has_legal_move`, only when the mover has
  <= 6 pieces.
- TT depth arithmetic is overflow-safe; compile-time check for native 64-bit atomics.
- Delta pruning is skipped when the mover has a pawn on its 7th rank. End to end it keeps the
  same best move in 18/21 test positions (mean score shift 27 cp) with 2.4x fewer nodes.
- `BoardState::make_null_move` (hash-consistent) replaces the inline null-move code.
New tests (66 per engine target): legality filter vs brute force on 6,000 random positions,
9 reference perft positions, 26 independently counted tricky positions (pins, ep, double
check, castling, promotions), stalemate/evasions/mate distance, null-move hash and
no-consecutive-null, repetition semantics, TT concurrency (8 threads x 300k ops), TT age wrap
and replacement, bounded history, Lazy SMP abort/reuse/time limits, aspiration recovery,
colour-symmetric evaluation on 3,000 random positions.
Tuning validation: `blunder_scan fit` now cross-validates BY GAME (4 folds). Mean |static - Stockfish|
on held-out games: old weights 284, shipped 245, fit 197 (the fit was NOT shipped: its head-to-head
score was 56% vs 75% for the shipped 50% weights).
`sf_match`: even game count (balanced colours), SF_ADJ_CP / SF_ADJ_PLIES / SF_NO_ADJ,
per-colour tallies, new-game handshake.
Regression check of this part vs the previous commit (300 ms/move, 44 games): 15W 12D 17L,
i.e. no measurable strength change (about 5-10% slower on the depth-7 benchmark).

## Tools added (Rust only)
- `sf_match <level 1-8|0> <think_ms> <games> [path]`: plays the engine in-process vs
  Stockfish at a Lichess AI level (or, with level 0, vs another UCI engine binary).
- `blunder_scan`: replays logged games with Stockfish (eval drops) and fits term weights
  (`blunder_scan fit <step> <depth> <logs...>`).

## Known remaining issues (not changed)
- UCI `go` is synchronous: `go infinite` cannot be stopped.
- Default Threads=4 regardless of core count; eval is not incremental; some eval terms
  untuned (mobility counts pawn-attacked squares if defended; duplicate material constants).
