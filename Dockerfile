# Builder stage
FROM rust:1.83-slim-bullseye as builder

ARG TRACE_LEVEL

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/atoma-proxy

COPY . .

# Build the application
RUN RUST_LOG=${TRACE_LEVEL} cargo build --release --bin atoma-proxy

# Final stage
FROM debian:bullseye-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl1.1 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Create necessary directories
RUN mkdir -p /app/logs

# Copy the built binary from builder stage
COPY --from=builder /usr/src/atoma-proxy/target/release/atoma-proxy /usr/local/bin/atoma-proxy

# Copy configuration file
COPY config.toml ./config.toml

# Set executable permissions explicitly
RUN chmod +x /usr/local/bin/atoma-proxy

# Copy and set up entrypoint script
COPY entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh

# Copy host client.yaml and modify keystore path
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

# Use full path in CMD
CMD ["/usr/local/bin/atoma-proxy", "--config-path", "/app/config.toml"]
