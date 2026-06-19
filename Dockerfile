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
# Cargo features for the cairn binaries. Default is empty (lean image, no in-process
# embeddings): users get the deterministic hashing embedder out of the box, or wire up a
# hosted provider via CAIRN_EMBED_PROVIDER=openai|ollama. Opt into the heavy
# `embed-local` feature (fastembed/ONNX) with:
#   docker build --build-arg CAIRN_FEATURES=embed-local .
ARG CAIRN_FEATURES=""
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
# ca-certificates: TLS for hosted providers. libgomp1: only present because the
# CAIRN_FEATURES=embed-local opt-in links against the ONNX runtime; in the lean default
# build it's a few hundred KB of unused but harmless deps. wget: the cairn healthcheck
# in docker-compose.yml uses `wget -q` to probe /api/health.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgomp1 wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home cairn
COPY --from=builder /app/target/release/cairn /usr/local/bin/cairn
COPY --from=builder /app/target/release/cairn-cli /usr/local/bin/cairn-cli
USER cairn
VOLUME ["/data"]
EXPOSE 7777
ENTRYPOINT ["cairn"]
CMD ["serve", "--host", "0.0.0.0", "--port", "7777", "--data-dir", "/data"]
