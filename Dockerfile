# Stage 1: Build (Performance Optimized)
FROM rust:1.85-slim-bookworm as builder

# Build dependencies install karo (SSL aur arithmetic libs ke liye)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# [OPTIMIZATION] Dependency Caching
# Isse build time 90% kam ho jayega har code change par
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Ab asli source code copy karo
COPY . .

# Dummy build ko remove karke asli build start karo
RUN touch src/main.rs && cargo build --release

# Stage 2: Runtime (Ultra-Lightweight for Speed)
FROM debian:bookworm-slim

# Runtime environment setup
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Builder stage se binary copy karo
COPY --from=builder /usr/src/app/target/release/the-sovereign-shadow /usr/local/bin/sovereign-shadow
RUN chmod +x /usr/local/bin/sovereign-shadow

# Bot start karne ki command
CMD ["sovereign-shadow"]