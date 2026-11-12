# syntax=docker/dockerfile:1

# 1. Define global build argument (1 = release, 0 = debug/dev)
ARG RELEASE_BUILD=1

FROM rust:slim-trixie AS build
# Bring the global ARG into this build stage
ARG RELEASE_BUILD

WORKDIR /src

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get --assume-yes install --no-install-recommends \
    ca-certificates \
    clang \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libclang-dev \
    libsqlite3-dev \
    libswresample-dev \
    libswscale-dev \
    npm \
    nodejs \
    pkg-config && \
    rm -rf /var/lib/apt/lists/*

# --- FRONTEND ---
# Install dependencies before the sources are copied, so the `npm ci` layer
# stays cached and only the build itself re-runs on frontend changes.
COPY package.json package-lock.json ./

RUN --mount=type=cache,target=/root/.npm \
    npm ci --no-audit --no-fund

COPY vite.config.ts tsconfig.json ./
COPY frontend ./frontend

RUN if [ "$RELEASE_BUILD" = "1" ]; then \
    echo "Building frontend in RELEASE mode" && \
    npm run build-only; \
    else \
    echo "Building frontend in DEBUG/DEV mode" && \
    npm run build-only -- --mode development; \
    fi

# --- BACKEND ---
ENV FFPLAYOUT_FRONTEND_PREBUILT=1 \
    CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16

COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY migrations ./migrations
COPY assets ./assets

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    if [ "$RELEASE_BUILD" = "1" ]; then \
    echo "Building backend in RELEASE mode" && \
    cargo build --release --locked --package ffplayout --no-default-features --features embed_frontend && \
    cp target/release/ffplayout /usr/local/bin/ffplayout; \
    else \
    echo "Building backend in DEBUG mode" && \
    cargo build --locked --package ffplayout --no-default-features --features embed_frontend && \
    cp target/debug/ffplayout /usr/local/bin/ffplayout; \
    fi

FROM debian:trixie

RUN apt-get update && \
    apt-get --assume-yes install --no-install-recommends \
    ca-certificates \
    ffmpeg \
    fonts-dejavu-core && \
    rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local/bin/ffplayout /usr/bin/ffplayout
COPY assets/dummy.vtt /usr/share/ffplayout/
COPY assets/logo.png /usr/share/ffplayout/

VOLUME ["/usr/share/ffplayout/db", "/usr/share/ffplayout/public", "/var/lib/ffplayout/tv-media", "/var/lib/ffplayout/playlists"]

EXPOSE 8787

ENTRYPOINT ["/usr/bin/ffplayout"]
CMD ["-l", "0.0.0.0:8787"]