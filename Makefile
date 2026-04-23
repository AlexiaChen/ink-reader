.PHONY: build test install clean

build:
	cargo clippy -- -D warnings
	cargo build --release

test:
	cargo test

install:
	cargo install --path . --root /usr/local

clean:
	cargo clean
