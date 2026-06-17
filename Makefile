CARGO ?= cargo

.PHONY: build install clean

build:
	$(CARGO) build --release

install:
	$(CARGO) install --path .

clean:
	$(CARGO) clean
