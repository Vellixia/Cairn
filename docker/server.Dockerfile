# cairn-server: the small central server behind shared project memory.
#
# Multi-stage: the Rust toolchain stays in the builder, and the final image
# carries the binary, the CA bundle and nothing else worth attacking.

# --- build ------------------------------------------------------------------
FROM rust:1.97.1-bookworm AS build

WORKDIR /src

# The toolchain file pins the compiler; copy it first so the pin is honoured.
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY tests ./tests

# Only the server is needed here. `--locked` keeps the build honest about the
# lockfile that was reviewed.
RUN cargo build --release --locked -p cairn-server \
    && strip target/release/cairn-server

# --- runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin cairn

COPY --from=build /src/target/release/cairn-server /usr/local/bin/cairn-server
COPY LICENSE /usr/share/doc/cairn/LICENSE

USER 10001:10001
EXPOSE 8080

# Bind to every interface: inside a container the loopback default would make
# the server unreachable from anywhere else in the network.
ENV CAIRN_SERVER_ADDR=0.0.0.0:8080

# `/api/health` is a real endpoint that answers without authentication.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

ENTRYPOINT ["/usr/local/bin/cairn-server"]
