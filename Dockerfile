# ── Build stage ─────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /usr/src/modelswitch

# Cache dependencies
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/bin/cli.rs && echo "" > src/lib.rs && \
    cargo build --bin modelswitch-cli --release --no-default-features 2>/dev/null || true

# Build actual binary
COPY src-tauri/src ./src
RUN touch src/bin/cli.rs src/lib.rs && \
    cargo build --bin modelswitch-cli --release --no-default-features

# ── Runtime stage ───────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash modelswitch

# Copy binary
COPY --from=builder /usr/src/modelswitch/target/release/modelswitch-cli /usr/local/bin/modelswitch

# Create config directory
RUN mkdir -p /home/modelswitch/.config/modelswitch && \
    chown -R modelswitch:modelswitch /home/modelswitch/.config

USER modelswitch
ENV MODELSWITCH_CREDENTIAL_STORE=file
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -f http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["modelswitch"]
CMD ["serve"]
