# syntax=docker/dockerfile:1
# ── build ──────────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p refrax-cli

# ── run ────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /data refrax \
 && mkdir -p /data && chown refrax /data
COPY --from=build /src/target/release/refrax /usr/local/bin/refrax
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

USER refrax
VOLUME /data
ENV REFRAX_KEY=/data/keypair.json \
    REFRAX_DATA_DIR=/data \
    REFRAX_LISTEN=0.0.0.0:7701
EXPOSE 7701
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["node", "start"]
