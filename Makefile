CARGO ?= cargo

.PHONY: build install test clean

# The repository is a Cargo workspace (virtual root manifest, members under
# crates/); every target below addresses the whole workspace, and `install`
# names the CLI crate because `cargo install --path .` needs a [package].
build:
	$(CARGO) build --release --workspace

install:
	$(CARGO) install --path crates/ss-magic

test:
	$(CARGO) test --workspace --locked

clean:
	$(CARGO) clean
