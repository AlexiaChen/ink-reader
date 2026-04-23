# When running under sudo, cargo (a rustup shim) needs HOME pointing to
# the original user's directory so it can find the configured toolchain.
ifdef SUDO_USER
    REAL_HOME := $(shell eval echo ~$(SUDO_USER))
    CARGO := HOME=$(REAL_HOME) $(REAL_HOME)/.cargo/bin/cargo
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
