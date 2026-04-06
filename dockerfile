FROM rust:1.92 as builder

WORKDIR /app
COPY Cargo.toml ./
COPY src ./src

RUN cargo build --release

FROM debian:trixie-slim

# Install minimal dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install workload-specific runtime packages from packages.txt
COPY packages.txt /tmp/packages.txt
RUN if grep -qvE '^\s*#|^\s*$' /tmp/packages.txt; then \
        apt-get update -qq && \
        grep -vE '^\s*#|^\s*$' /tmp/packages.txt | xargs apt-get install -y -qq && \
        rm -rf /var/lib/apt/lists/*; \
    fi && rm -f /tmp/packages.txt

WORKDIR /app
COPY --from=builder /app/target/release/saturator /app/saturator

# Create directories
RUN mkdir -p /tmp /app/output

ENV PATH="/app/scripts:${PATH}" \
    LD_LIBRARY_PATH="/app/scripts/lib:${LD_LIBRARY_PATH}"

ENTRYPOINT ["/bin/sh", "-c", "umask 0000 && /app/saturator \"$@\"", "--"]
