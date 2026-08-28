FROM rust:1.98-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY sql/sqlite_schema.sql ./sql/sqlite_schema.sql
RUN cargo build --locked --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aprendiendo-mcp /usr/local/bin/aprendiendo-mcp

RUN mkdir /data && chown 65532:65532 /data
USER 65532:65532
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/aprendiendo-mcp"]
