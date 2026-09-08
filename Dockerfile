# syntax=docker/dockerfile:1
FROM rust:1.90-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/searchworks-mcp /usr/local/bin/searchworks-mcp
USER 65532:65532
EXPOSE 3000
ENV BIND_ADDRESS=0.0.0.0:3000 \
    RUST_LOG=searchworks_mcp=info,tower_http=info
ENTRYPOINT ["/usr/local/bin/searchworks-mcp"]
