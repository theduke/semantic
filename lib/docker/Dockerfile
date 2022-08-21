# FROM debian as builder

# SHELL ["/bin/bash", "-c"]

# RUN apt-get update && \
#     apt-get install -y curl unzip build-essential sassc binaryen git sudo && \
#     echo "Install Rust toolchain" && \
#     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain nightly -y && \
#     source $HOME/.cargo/env && \
#     rustup install nightly && rustup default nightly && \
#     rustup target add wasm32-unknown-unknown && \
#     echo "Install wasm-pack" && \
#     curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && \
#     echo "Install deno" && \
#     curl -fsSL https://deno.land/x/install/install.sh | DENO_INSTALL=/usr/bin sh && \
#     echo "Install mozjpeg" && \
#     curl -fsSL https://gist.githubusercontent.com/vikas5914/4affccdfa5c3a722808cd465f77866b9/raw/082f607ec1082aa63f258dbb11861112b7fd702d/build.sh | sh && \
#     mv /usr/bin/mozcjpeg /usr/bin/mozjpeg

# COPY . /build
# WORKDIR /build
# ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
# RUN cargo xtask build

# FROM debian

# COPY --from=builder /build/target/release/semantic /usr/bin/semantic

# ENV SEMANTIC_ADDRESS=0.0.0.0:80

# ENV SEMANTIC_DATA_PATH=/semantic
# VOLUME /semantic

# ENTRYPOINT [ "/usr/bin/semantic" ]

FROM lukemathwalker/cargo-chef:latest-rust-bullseye AS planner
COPY . /app
WORKDIR /app
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN cargo chef prepare --recipe-path ./recipe.json

FROM lukemathwalker/cargo-chef:latest-rust-bullseye AS builder

# Install OS build dependencies.
RUN apt-get update && \
  apt-get install -y make cmake protobuf-compiler libzstd-dev curl unzip build-essential clang sassc binaryen git sudo
 
RUN echo "Install wasm-pack" && curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

RUN rustup target add wasm32-unknown-unknown

WORKDIR /app
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
COPY . .
RUN git restore .cargo/config.toml

# Use cargo-chef to build a docker layer that contains just the compiled 
# dependencies.
# Allows future builds to skip recompiling the dependencies.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook -p semantic_cli --release --recipe-path recipe.json
RUN cargo chef cook -p semantic_ui --release --target wasm32-unknown-unknown --recipe-path recipe.json 

# Build UI
RUN cd crates/ui && wasm-pack build --target web --out-dir /app/target/ui
RUN cp crates/ui/index.html target/ui/
RUN sassc crates/ui/assets/styles.scss target/ui/styles.css
RUN mkdir target/ui/webfonts && cp crates/ui/assets/fontawesome-free-5.15.4-web/webfonts/* target/ui/webfonts/
RUN ls target/ui

# Build final binary
RUN ls -lha target && ls -lha target/release
RUN cargo xtask build
RUN cargo build -p semantic_cli --release && ls target/release

FROM debian:bullseye AS runtime
RUN ls /app/target/release
COPY --from=builder /app/target/release/semantic /usr/bin/semantic

EXPOSE 3000/tcp

CMD ["/usr/local/bin/semantic", "server"]
