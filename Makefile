.PHONY: build check fmt test chart docker-build docker-run

build:
	cargo build

check:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-features

fmt:
	cargo fmt

test:
	cargo test

chart:
	helm lint charts/searchworks-mcp --set image.tag=dev
	helm template searchworks-mcp charts/searchworks-mcp --set image.tag=dev > /dev/null

docker-build:
	docker build -t searchworks-mcp:dev .

docker-run:
	docker run --rm -p 3000:3000 searchworks-mcp:dev
