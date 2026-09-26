LIB      := target/release/libexecfilter.so
HOOKS    := execve|execv|execvp|execvpe|execl|execlp|posix_spawn|posix_spawnp
PREFIX   ?= /usr/local
DESTDIR  ?=

.PHONY: build test fmt clippy symbols install clean

build:
	cargo build --release

fmt:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

test: fmt clippy
	cargo test

# All 8 hooks must be exported for LD_PRELOAD interposition to work.
symbols: build
	@count=$$(nm -D --defined-only $(LIB) | grep -cE ' T ($(HOOKS))$$'); \
	echo "hooks exported: $$count/8"; \
	test "$$count" -eq 8

install: build
	install -Dm755 $(LIB) $(DESTDIR)$(PREFIX)/lib/execfilter.so

clean:
	cargo clean
