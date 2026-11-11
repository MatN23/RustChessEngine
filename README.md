# Rust Chess Engine

This repository contains a Rust chess engine with an experimental search/evaluation stack, Python bindings, a UCI engine binary, and a native Lichess bot. The code is actively evolving; the current structure reflects a more recent engine and bot implementation than the older README versions.

## What is in this repo

The project includes multiple entry points:

- `src/main.rs` -> UCI engine binary (`chess_uci`)
- `src/lichess_main.rs` -> native Lichess bot (`lichess_bot`)
- `src/lib.rs` -> Python extension module, enabled by the `python` feature
- `src/sf_match.rs` -> Stockfish comparison/match runner
- `src/blunder_scan.rs` -> analysis tool for Stockfish-labelled blunders

## Current engine features

The current engine is a Rust implementation with:

- board representation and move generation
- legal move generation and attack detection
- transposition-table based search
- alpha-beta style search with move ordering and pruning heuristics
- evaluation using material, piece-square tables, king safety, mobility, pawn structure, and tactical checks
- repetition and terminal position handling
- UCI option support for hash size, threads, multipv, and contempt
- Python access through `PyChessEngine` and `PyBoardState`

The code is not a thin wrapper around Stockfish; it is a native Rust engine using its own search and evaluation logic.

## Build requirements

- Rust 1.70+
- Python 3.8+ if using the Python extension
- pip
- `maturin` for Python builds

## Building

Default build includes the Python extension feature:

```bash
cargo build --release
```

Python bindings:

```bash
git clone https://github.com/MatN23/RustChessEngine.git
cd RustChessEngine
python3 -m pip install maturin
maturin develop --release --features python
```

Standalone UCI binary without the Python feature:

```bash
cargo build --release --bin chess_uci --no-default-features
```

Native Lichess bot:

```bash
cargo build --release --bin lichess_bot --no-default-features
```

## Running the engine

### UCI engine

```bash
cargo run --release --bin chess_uci
```

The UCI binary implements standard commands such as:

- `uci`
- `isready`
- `ucinewgame`
- `position ...`
- `go ...`
- `stop`
- `quit`
- `d`
- `perft <depth>`
- `evalterms`

It exposes UCI options including:

- `Hash`
- `Threads`
- `MultiPV`
- `Contempt`
- `ClearHash`

### Python

```python
import chess_engine

engine = chess_engine.PyChessEngine(threads=4)
result = engine.search(
    fen="rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    depth=12,
    time_ms=5000,
)

print(result)
```

The Python API exposes:

- `PyChessEngine.new(threads=None)`
- `search(fen, depth=None, time_ms=None)`
- `new_game()`
- `set_threads(n)`
- `set_multi_pv(n)`
- `set_hash_size(mb)`
- `set_contempt(value)`
- `stop()`
- `PyBoardState` helpers for FEN parsing and move application

### Lichess bot

```bash
LICHESS_TOKEN=YOUR_TOKEN cargo run --release --no-default-features --bin lichess_bot -- YOUR_TOKEN
```

The bot reads configuration from environment variables and CLI arguments, and it is designed to connect directly to Lichess with its own game loop and timing logic rather than delegating to a Python subprocess.

## Additional tools

### Stockfish match runner

```bash
cargo run --release --bin sf_match -- 5 200 10 /usr/games/stockfish 1 512
```

This plays the engine against Stockfish for benchmarking and match analysis.

### Blunder scanner

```bash
cargo run --release --bin blunder_scan -- <logfile> <game_no> <W|B> [sf_depth] [engine_ms] [threshold_cp]
```

This reads a game log and reports big evaluation swings compared to Stockfish.

## Repo layout

```text
RustChessEngine/
|-- Cargo.toml
|-- README.md
|-- LICENSE
|-- src/
|   |-- lib.rs
|   |-- main.rs
|   |-- lichess_main.rs
|   |-- board.rs
|   |-- bitboard.rs
|   |-- movegen.rs
|   |-- search.rs
|   |-- eval.rs
|   |-- uci.rs
|   |-- zobrist.rs
|   |-- opening_book.rs
|   |-- sf_match.rs
|   |-- blunder_scan.rs
|   |-- lichess/
|   |   |-- bot.rs
|   |   |-- client.rs
|   |   |-- game.rs
|   |   |-- timing.rs
|   |   |-- tests.rs
|   |   `-- ...
|-- build_and_install.sh
|-- debug_build.sh
|-- test_engine.py
`-- .gitignore
```

## Testing

```bash
cargo test
```

At the moment this project is best viewed as an active engine-and-bot codebase rather than a finished production chess engine. The main targets are the native engine, the UCI interface, the Python bindings, and the Lichess bot loop.

## Acknowledgements

- [Stockfish](https://github.com/official-stockfish/Stockfish)
- [Chess Programming Wiki](https://www.chessprogramming.org/)
- [PyO3](https://github.com/PyO3/pyo3)
