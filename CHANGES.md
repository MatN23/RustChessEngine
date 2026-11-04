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

## Known remaining issues (not changed)
- UCI `go` is synchronous: `go infinite` cannot be stopped.
- Default Threads=4 regardless of core count; eval is not incremental; some eval terms
  untuned (mobility counts pawn-attacked squares if defended; duplicate material constants).
