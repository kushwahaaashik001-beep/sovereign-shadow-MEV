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

# Copy Cargo.toml and Cargo.lock first to leverage Docker cache for dependencies.
# This layer will only be invalidated if these files change.
COPY Cargo.toml Cargo.lock ./

# Create a dummy src/main.rs to allow `cargo check` to run and cache dependencies.
# This step will download and compile all dependencies without building the main application.
RUN mkdir -p src && \
    echo "fn main() { println!(\"Caching dependencies...\"); }" > src/main.rs && \
    cargo check --release && \
    rm src/main.rs

# Copy the rest of the application source code. This layer is invalidated if source code changes.
COPY . .

# Build the release binary for your project. This will use the cached dependencies.
RUN cargo build --release --bin the-sovereign-shadow

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