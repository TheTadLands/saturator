FROM rust:1.92 as builder

WORKDIR /app
COPY Cargo.toml ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

# Install minimal dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/saturator /app/saturator

# Create directories
RUN mkdir -p /tmp /app/output

ENTRYPOINT ["/app/saturator"]