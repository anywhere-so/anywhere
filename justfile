set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

build:
    cargo build --release --locked -p anywhere

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    shellcheck cli/install.sh

test:
    cargo test --workspace --locked

install-test:
    python3 cli/test_install.py

schema-test:
    python3 -m unittest discover -s tools -p 'test_*.py'

notices:
    python3 tools/notices.py

notices-check:
    python3 tools/notices.py --check

check: fmt-check lint test install-test schema-test notices-check

fuzz target:
    MISE_ENV=fuzz mise exec -- cargo fuzz run {{target}}
