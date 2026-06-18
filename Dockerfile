# syntax=docker/dockerfile:1

# ---- web build: produce the static UI export ---------------------------------
FROM node:22-bookworm AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build   # -> /web/out

# ---- rust build: compile the CLI, embedding the UI ---------------------------
FROM rust:1-bookworm AS builder
# Cargo features for the cairn binaries. Defaults to local embeddings (all-MiniLM-L6-v2); override
# with --build-arg CAIRN_FEATURES="" for a lean image that uses a hosted embedding provider.
ARG CAIRN_FEATURES=embed-local
WORKDIR /app
COPY . .
# Bring in the freshly built UI so cairn-api can embed it.
COPY --from=web /web/out ./web/out
RUN if [ -n "$CAIRN_FEATURES" ]; then \
        cargo build --release -p cairn-server -p cairn-cli --features "$CAIRN_FEATURES"; \
    else \
        cargo build --release -p cairn-server -p cairn-cli; \
    fi

# ---- runtime -----------------------------------------------------------------
FROM debian:bookworm-slim
# ca-certificates: TLS for hosted providers / model downloads. libgomp1: OpenMP runtime that the
# ONNX Runtime (local embeddings) links against.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home cairn
COPY --from=builder /app/target/release/cairn /usr/local/bin/cairn
COPY --from=builder /app/target/release/cairn-cli /usr/local/bin/cairn-cli
USER cairn
VOLUME ["/data"]
EXPOSE 7777
ENTRYPOINT ["cairn"]
CMD ["serve", "--host", "0.0.0.0", "--port", "7777", "--data-dir", "/data"]
