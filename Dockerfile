# Stage 1: Build
# trixie (not bookworm): the prebuilt static onnxruntime that ort-sys
# downloads is built against glibc >= 2.38 and gcc-13 libstdc++
# (__isoc23_strtoll, __cxa_call_terminate) — bookworm's glibc 2.36/gcc-12
# cannot link it.
FROM rust:1.93-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

# onnx embeddings are a default feature of oi-cli — without them search is
# semantically random, so no feature flags needed here.
# -lstdc++: ort-sys statically links onnxruntime's C++ objects; the final
# rust link needs the C++ runtime spelled out on this toolchain.
RUN RUSTFLAGS="-C link-arg=-lstdc++" cargo build --release -p oi-cli --bin oi

# Stage 2: Runtime (must match builder's glibc generation)
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

# Run as non-root user
RUN useradd -m -u 1000 -s /usr/sbin/nologin oi

COPY --from=builder /build/target/release/oi /usr/local/bin/oi

USER oi

# Index, stakes, and config live here; model cache downloads on first run —
# mount volumes to persist both.
VOLUME ["/home/oi/.opensonarx", "/home/oi/.cache"]

# 4001 P2P (TCP+QUIC), 8001 HTTP API
EXPOSE 4001/tcp 4001/udp 8001/tcp

ENTRYPOINT ["oi"]
