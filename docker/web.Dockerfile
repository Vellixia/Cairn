# cairn-web: the browser UI for reading and managing shared project memory.
#
# Next.js `output: "standalone"` means the runtime stage needs the server
# bundle and its traced dependencies, not the whole node_modules tree.

# --- build ------------------------------------------------------------------
FROM node:22-bookworm-slim AS build

WORKDIR /app

COPY web/package.json web/package-lock.json ./
RUN npm ci

COPY web/ ./

# The API origin is configured at *run* time, not here: set CAIRN_API_ORIGIN in
# the container's environment and the root layout serves it to the browser on
# every request. One published image therefore works behind any hostname.
#
# This build arg remains for the two cases that still want a baked origin — a
# private image for a fixed deployment, and the e2e job, which serves the UI and
# the API on different ports. Left empty the bundle resolves to the same origin
# that served the page, which is the recommended layout (web at `/`, API at
# `/api` behind one hostname).
ARG NEXT_PUBLIC_CAIRN_API=
ENV NEXT_PUBLIC_CAIRN_API=${NEXT_PUBLIC_CAIRN_API}
ENV NEXT_TELEMETRY_DISABLED=1

RUN npm run build

# --- runtime ----------------------------------------------------------------
FROM node:22-bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3100
ENV HOSTNAME=0.0.0.0

COPY --from=build /app/.next/standalone ./
COPY --from=build /app/.next/static ./.next/static
COPY LICENSE /usr/share/doc/cairn/LICENSE

# `node` already exists in this image as uid 1000.
USER node
EXPOSE 3100

HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3100/ || exit 1

CMD ["node", "server.js"]
