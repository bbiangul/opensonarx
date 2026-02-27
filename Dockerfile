# Stage 1: Build
FROM rust:1.90-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release -p oi-cli --bin oi

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

# Run as non-root user
RUN useradd -m -u 1000 -s /usr/sbin/nologin oi

COPY --from=builder /build/target/release/oi /usr/local/bin/oi

USER oi

EXPOSE 4001/tcp 4001/udp

ENTRYPOINT ["oi"]
