# Rust Chess Engine

A Rust chess engine with Python bindings and a UCI-compatible binary. The project is designed for local chess-engine development, experimentation, and integration with external tooling.

## Features

- Alpha-beta search with transposition table support
- Move ordering and pruning heuristics
- Evaluation based on material, mobility, king safety, and pawn structure
- Python bindings via PyO3
- UCI binary for GUI and CLI use
- Lichess bot support

## Requirements

- Rust 1.70+
- Python 3.8+
- pip
- maturin for Python builds

## Installation

```bash
git clone https://github.com/MatN23/RustChessEngine.git
cd RustChessEngine
python3 -m pip install maturin
maturin develop --release --features python
```

For the standalone UCI binary:

```bash
cargo build --release --bin chess_uci --no-default-features
```

## Usage

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

### UCI

```bash
./target/release/chess_uci
```

### Lichess bot

```bash
cargo build --release --bin lichess_bot
LICHESS_TOKEN=YOUR_TOKEN ./target/release/lichess_bot
```

## Project layout

```text
RustChessEngine/
|---- src/
|   |---- lib.rs
|   |---- main.rs
|   |---- board.rs
|   |---- bitboard.rs
|   |---- eval.rs
|   |---- movegen.rs
|   |---- search.rs
|   |---- uci.rs
|   |---- lichess_main.rs
|   `---- ...
|---- Cargo.toml
|---- README.md
|---- build_and_install.sh
|---- debug_build.sh
|---- test_engine.py
|---- LICENSE
`---- .gitignore
```

## Development

```bash
cargo test
```

## License

MIT License. See the LICENSE file for details.

## Acknowledgements

- [Stockfish](https://github.com/official-stockfish/Stockfish)
- [Chess Programming Wiki](https://www.chessprogramming.org/)
- [PyO3](https://github.com/PyO3/pyo3)
