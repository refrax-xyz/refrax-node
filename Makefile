.PHONY: help build test lint fmt devnet demo-svg docker

help:            ## list targets
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[38;5;141m%-10s\033[0m %s\n", $$1, $$2}'

build:           ## release build of the `refrax` binary
	cargo build --release -p refrax-cli

test:            ## unit tests + 16-node devnet tests
	cargo test --workspace

lint:            ## clippy with warnings as errors, format check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all --check

fmt:             ## format the code
	cargo fmt --all

devnet: build    ## registry + 16 nodes on localhost
	./target/release/refrax devnet up

demo-svg:        ## re-render .github/assets/demo.svg
	python3 scripts/demo-svg.py

docker:          ## build the node image
	docker build -t refrax-node .
