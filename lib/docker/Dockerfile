FROM debian as builder

SHELL ["/bin/bash", "-c"]

RUN apt-get update && \
    apt-get install -y curl unzip build-essential sassc binaryen git sudo && \
    echo "Install Rust toolchain" && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y && \
    source $HOME/.cargo/env && \
    rustup target add wasm32-unknown-unknown && \
    echo "Install wasm-pack" && \
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && \
    echo "Install deno" && \
    curl -fsSL https://deno.land/x/install/install.sh | DENO_INSTALL=/usr/bin sh && \
    echo "Install mozjpeg" && \
    curl -fsSL https://gist.githubusercontent.com/vikas5914/4affccdfa5c3a722808cd465f77866b9/raw/082f607ec1082aa63f258dbb11861112b7fd702d/build.sh | sh && \
    mv /usr/bin/mozcjpeg /usr/bin/mozjpeg

COPY . /build
WORKDIR /build
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN cargo xtask build

FROM debian

# COPY --from=builder /build/target/release/semantic /usr/bin/semantic

# ENV SEMANTIC_ADDRESS=0.0.0.0:80

# ENV SEMANTIC_DATA_PATH=/semantic
# VOLUME /semantic

# ENTRYPOINT [ "/usr/bin/semantic" ]
