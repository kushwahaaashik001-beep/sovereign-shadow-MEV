# --- Build Stage ---
FROM rust:latest as builder

# Install system dependencies for compilation (needed for OpenSSL/crypto)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# Pillar INFRA: Force lockfile update to sync with the latest stable toolchain and resolve version conflicts
RUN cargo update

# Build with release profile for nanosecond performance
RUN cargo build --release

# --- Runtime Stage ---
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/the-sovereign-shadow .

# Ensure binary is executable for Hugging Face
RUN chmod +x the-sovereign-shadow

ENTRYPOINT ["./the-sovereign-shadow"]