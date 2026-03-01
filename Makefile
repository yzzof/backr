LINUX   = x86_64-unknown-linux-gnu
WINDOWS = x86_64-pc-windows-gnu

.PHONY: build release clean

build:
	cargo build --target $(LINUX)
	cross build --target $(WINDOWS)

release:
	cargo build --release --target $(LINUX)
	cross build --release --target $(WINDOWS)

clean:
	cargo clean
