FROM rust:1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY static ./static
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates tini curl \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 10001 --user-group --no-create-home --shell /usr/sbin/nologin clips \
    && mkdir -p /data && chown 10001:10001 /data
COPY --from=builder /build/target/release/fthr-clips-server /usr/local/bin/fthr-clips-server
USER 10001:10001
ENV DATA_DIR=/data BIND_HOST=0.0.0.0 PORT=8080
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["tini", "--"]
CMD ["fthr-clips-server", "serve"]
