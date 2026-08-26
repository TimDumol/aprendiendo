FROM rust:1.98-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aprendiendo-mcp /usr/local/bin/aprendiendo-mcp

USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/aprendiendo-mcp"]

