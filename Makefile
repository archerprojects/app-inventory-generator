.PHONY: build deb clean

build:
	cargo build --release

deb: build
	mkdir -p dist
	cargo deb --output dist/

clean:
	cargo clean
	rm -rf dist
