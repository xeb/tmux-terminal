#!/bin/bash
cd "$(dirname "$0")"

# Kill any existing process on port 5533
lsof -ti:5533 | xargs kill -9 2>/dev/null

cargo build --release && ./target/release/tmux-terminal
