# --- STAGE 1: BUILD ENGINE ---
FROM rust:1.85-slim-bookworm AS builder

# Install build-essential tools for high-performance crates (secp2k1, alloy)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    cmake \
    git \
    protobuf-compiler \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .

# Build with high-level optimization (Release Mode)
# Cargo.toml already has strip = true and lto = "fat" for nanosecond edge.
RUN cargo build --release

# --- STAGE 2: THE FORTRESS RUNTIME ---
FROM debian:bookworm-slim

# Install minimal runtime dependencies (SSL for RPC, CA-Certs for HTTPS)
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /usr/src/app/target/release/the-sovereign-shadow /app/

# Pillar HF: Ensure the bot listens on the port provided by Hugging Face
ENV PORT=7860
EXPOSE 7860

# Pillar MODE: Support for Double Space architecture (Space A: scout / Space B: sniper)
ENV MODE=sniper

CMD ["sh", "-c", "./the-sovereign-shadow ${MODE}"]