#!/bin/bash

set -e

echo "Building Rust Chess Engine"
echo "==========================="
echo ""

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Rust is not installed.${NC}"
    echo "Install from: https://rustup.rs"
    exit 1
fi

echo -e "${GREEN}Rust found: $(rustc --version)${NC}"

if ! command -v python3 &> /dev/null; then
    echo -e "${RED}Python3 is not installed.${NC}"
    exit 1
fi

echo -e "${GREEN}Python found: $(python3 --version)${NC}"

if ! command -v maturin &> /dev/null; then
    echo -e "${YELLOW}Installing maturin...${NC}"
    python3 -m pip install maturin
fi

echo -e "${GREEN}Maturin found: $(maturin --version)${NC}"
echo ""

echo "Choose build option:"
echo "  1) Python module only"
echo "  2) UCI binary only"
echo "  3) Both"
echo ""
read -p "Enter choice [1-3]: " choice

case $choice in
    1)
        echo ""
        echo -e "${YELLOW}Building Python module...${NC}"
        RUSTFLAGS="-C target-cpu=native" maturin develop --release --features python
        if [ $? -eq 0 ]; then
            echo ""
            echo -e "${GREEN}Python module built successfully.${NC}"
            echo ""
            echo "Test with:"
            echo "  python3 -c \"import chess_engine; e = chess_engine.PyChessEngine(); print(e)\""
        fi
        ;;
    2)
        echo ""
        echo -e "${YELLOW}Building UCI binary...${NC}"
        RUSTFLAGS="-C target-cpu=native" cargo build --release --bin chess_uci --no-default-features
        if [ $? -eq 0 ]; then
            echo ""
            echo -e "${GREEN}UCI binary built successfully.${NC}"
            echo ""
            echo "Binary location: ./target/release/chess_uci"
            echo "Run with: ./target/release/chess_uci"
        fi
        ;;
    3)
        echo ""
        echo -e "${YELLOW}Building Python module...${NC}"
        RUSTFLAGS="-C target-cpu=native" maturin develop --release --features python

        echo ""
        echo -e "${YELLOW}Building UCI binary...${NC}"
        RUSTFLAGS="-C target-cpu=native" cargo build --release --bin chess_uci --no-default-features

        if [ $? -eq 0 ]; then
            echo ""
            echo -e "${GREEN}All components built successfully.${NC}"
            echo ""
            echo "Python module: import chess_engine"
            echo "UCI binary: ./target/release/chess_uci"
        fi
        ;;
    *)
        echo -e "${RED}Invalid choice.${NC}"
        exit 1
        ;;
esac

echo ""
echo "Build complete."
echo ""
echo "Next steps:"
echo "  - Test Python: python3 test_engine.py"
echo "  - Run Lichess bot: LICHESS_TOKEN=YOUR_TOKEN ./target/release/lichess_bot"
echo "  - Use UCI: ./target/release/chess_uci"
echo ""
