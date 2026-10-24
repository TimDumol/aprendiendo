FROM rust:1.98-trixie AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY apps/practice/lib/domain/dele-tasks.json ./apps/practice/lib/domain/dele-tasks.json
COPY sql/sqlite_schema.sql ./sql/sqlite_schema.sql
COPY sql/taxonomy_migration.sql ./sql/taxonomy_migration.sql
RUN cargo build --locked --release

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aprendiendo-mcp /usr/local/bin/aprendiendo-mcp
COPY --from=builder /build/target/release/practice_worker /usr/local/bin/practice_worker

COPY --from=builder /build/target/release/migrate /usr/local/bin/migrate
COPY --from=builder /build/target/release/audit_data /usr/local/bin/audit_data

RUN mkdir /data && chown 65532:65532 /data
USER 65532:65532
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/aprendiendo-mcp"]
