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
WORKDIR /app
COPY . .
# Bring in the freshly built UI so cairn-api can embed it.
COPY --from=web /web/out ./web/out
RUN cargo build --release -p cairn-cli

# ---- runtime -----------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home cairn
COPY --from=builder /app/target/release/cairn /usr/local/bin/cairn
USER cairn
VOLUME ["/data"]
EXPOSE 7777
ENTRYPOINT ["cairn"]
CMD ["serve", "--host", "0.0.0.0", "--port", "7777", "--data-dir", "/data"]
