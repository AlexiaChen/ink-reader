# When running under sudo, locate cargo via the original user's home dir.
ifdef SUDO_USER
    CARGO := $(shell eval echo ~$(SUDO_USER))/.cargo/bin/cargo
else
    CARGO := $(shell command -v cargo 2>/dev/null || echo $$HOME/.cargo/bin/cargo)
endif

.PHONY: build test install clean

build:
	$(CARGO) clippy -- -D warnings
	$(CARGO) build --release

test:
	$(CARGO) test

install:
	$(CARGO) install --path . --root /usr/local

clean:
	$(CARGO) clean
