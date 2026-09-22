# Stage 0: common build base with mirrors + cargo-chef
FROM rust:1.97-alpine AS chef
RUN sed -i 's|dl-cdn.alpinelinux.org|mirrors.tuna.tsinghua.edu.cn|g' /etc/apk/repositories && \
    apk add --no-cache musl-dev gcc
# Cargo mirror for China (build-network optimization); gcc compiles ring's
# C code (rustls's crypto backend).
RUN mkdir -p $CARGO_HOME && \
    printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n[net]\ngit-fetch-with-cli = true\n' > $CARGO_HOME/config.toml
RUN cargo install cargo-chef
WORKDIR /app

# Stage 1: Generate a recipe file from all dependencies
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Build and cache dependencies using the recipe
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

# Stage 3: Copy real source and build (deps already cached from stage 2)
COPY . .
RUN cargo build --release --locked -p hut_email && \
    cp target/release/hut_email /usr/local/bin/hut_email

# Stage 4: Minimal runtime image
FROM alpine:3.21

# rustls ships its own webpki-roots, so no system CA bundle or libssl needed.
RUN addgroup -S app && adduser -S app -G app

WORKDIR /app
COPY --from=builder /usr/local/bin/hut_email /usr/local/bin/hut_email

USER app

EXPOSE 39788

CMD ["hut_email"]
