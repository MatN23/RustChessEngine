#!/bin/bash

echo "Building Rust Chess Engine..."
echo "=============================="

# Install maturin if not installed
if ! command -v maturin &> /dev/null; then
    echo "Installing maturin..."
    pip install maturin
fi

# Build the Rust extension in release mode
echo "Compiling Rust code (this may take a few minutes)..."
maturin develop --release

if [ $? -eq 0 ]; then
    echo ""
    echo "✓ Build successful!"
    echo ""
    echo "To test the engine:"
    echo "  python -c 'import chess_engine; print(chess_engine)'"
    echo ""
    echo "To run the Lichess bot:"
    echo "  python lichess_bot_rust.py YOUR_TOKEN"
    echo ""
else
    echo ""
    echo "✗ Build failed. Please check the error messages above."
    exit 1
fi