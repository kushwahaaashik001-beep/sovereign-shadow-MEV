# --- Build Stage ---
FROM rust:1.85-slim-bookworm as builder

# Install system dependencies for compilation (needed for OpenSSL/crypto)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

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