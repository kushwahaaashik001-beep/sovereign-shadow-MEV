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

# Pillar HF: Resolve Dependency Version Conflicts
# Some crates (serde_with, icu) are requesting futuristic Rust versions (1.86/1.88).
# We force them to the last stable versions compatible with Rust 1.85.
RUN cargo update -p serde_with --precise 3.11.0 || true && \
    cargo update -p serde_with_macros --precise 3.11.0 || true && \
    cargo update -p icu_normalizer_data --precise 1.5.0 || true && \
    cargo update -p icu_properties --precise 1.5.0 || true && \
    cargo update -p icu_properties_data --precise 1.5.0 || true && \
    cargo update -p icu_provider --precise 1.5.0 || true

# Build with memory limits to prevent Hugging Face OOM
ENV PROTOC_NO_VENDOR=1
RUN CARGO_NET_GIT_FETCH_WITH_CLI=true \
    cargo build --release --jobs 1

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

# Pillar MODE: Support for Double Space architecture
ENV MODE=sniper

CMD ["sh", "-c", "./the-sovereign-shadow ${MODE}"]