FROM rust:latest AS builder

# Install required dependencies
RUN apt-get update && apt-get install -y \
    cmake \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /app

# Copy the Cargo.toml and Cargo.lock files to build dependencies
COPY Cargo.toml Cargo.lock ./

# Create a dummy source file to force build dependencies
RUN mkdir src && echo "fn main() { println!(\"Hello, world!\"); }" > src/main.rs

# Build the dependencies for caching
RUN cargo build --release && rm -rf src

# Copy the source code
COPY . .

# Build the application
RUN cargo build --release

# Use a minimal runtime image with newer glibc
FROM debian:bookworm-slim

# Install necessary runtime libraries
RUN apt-get update && apt-get install -y libssl-dev libclang-dev && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /app

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/grpc-server app


# Command to run the application
CMD ["./app"]
