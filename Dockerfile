# Stage 1: Build stage
FROM rust:1.88-bookworm as builder

# Saare possible tools jo blockchain crates ko chahiye hote hain
RUN apt-get update && apt-get install -y \
    clang \
    llvm-dev \
    libclang-dev \
    cmake \
    pkg-config \
    libssl-dev \
    build-essential \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Libclang path set karna zaroori hai c-kzg ke liye
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

WORKDIR /usr/src/app
COPY . .

# Build the release binary
RUN cargo build --release

# Stage 2: Final Light image
FROM debian:bookworm-slim

# Runtime libraries install karo
RUN apt-get update && apt-get install -y \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Binary copy karo (Check karna binary ka naam 'the-sovereign-shadow' hi hai na)
COPY --from=builder /usr/src/app/target/release/the-sovereign-shadow /usr/local/bin/bot

# Bot start command
CMD ["bot"]