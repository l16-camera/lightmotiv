.PHONY: build release release-fast install bench check lumen lumen-release package-macos version version-bump version-check libcp-export

LIBCP_EXPORT_SRC := tools/libcp-export/libcp_export.cpp
LIBCP_EXPORT_BIN := tools/libcp-export/libcp-export

version:
	@./scripts/calver show

version-bump:
	@./scripts/calver bump

version-check:
	@./scripts/calver check

build:
	cargo build -p light

release:
	cargo build --release -p light

lumen:
	cargo build -p lumen

lumen-release:
	cargo build --release -p lumen

# Double-clickable dist/Luminat.app (+ embedded libcp-export)
package-macos:
	@./scripts/package-macos.sh

release-fast:
	cargo build --profile release-fast -p light

install:
	cargo install --path light --force

bench:
	cargo bench -p lri-rs

check:
	cargo check --workspace

# x86_64 helper for Light libcp.dylib (Rosetta). Does not ship proprietary dylibs.
# Usage: light libcp --lri photo.lri -o ./out
libcp-export: $(LIBCP_EXPORT_BIN)

$(LIBCP_EXPORT_BIN): $(LIBCP_EXPORT_SRC)
	clang++ -arch x86_64 -std=c++14 -O2 -o $@ $< \
		-Wl,-rpath,@loader_path \
		-Wl,-rpath,@loader_path/vendor
	@file $@