.PHONY: build-cli build-release build-tauri docker-build install lint test clean check

# ── Build targets ──────────────────────────────────────────

build-cli:
	cargo build --bin modelswitch-cli --no-default-features

build-release:
	cargo build --bin modelswitch-cli --release --no-default-features

build-tauri:
	cargo build --features tauri

# ── Docker ─────────────────────────────────────────────────

docker-build:
	docker build -t modelswitch:latest .

docker-run: docker-build
	docker run -d --name modelswitch \
		-p 8080:8080 \
		-v $$(pwd)/config.toml:/etc/modelswitch/config.toml:ro \
		modelswitch:latest

# ── Install ────────────────────────────────────────────────

install: build-release
	cp target/release/modelswitch-cli /usr/local/bin/modelswitch
	@echo "Installed to /usr/local/bin/modelswitch"

install-service: install
	cp deploy/modelswitch.service /etc/systemd/system/
	systemctl daemon-reload
	systemctl enable modelswitch
	@echo "Service installed. Run: systemctl start modelswitch"

# ── Quality ────────────────────────────────────────────────

lint:
	cargo clippy --all-features -- -D warnings
	cargo fmt --check

test:
	cargo test --all-features

check:
	cargo check --bin modelswitch-cli --no-default-features
	cargo check --features tauri

clean:
	cargo clean
