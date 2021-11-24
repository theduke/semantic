FROM debian as builder

SHELL ["/bin/bash", "-c"]

RUN apt-get update && \
    apt-get install -y curl && \
    echo "Install Rust toolchain" && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y && \
    source $HOME/.cargo/env && \
    rustup target add wasm32-unknown-unknown && \
    echo "Install wasm-pack" && \
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && \
    echo "Install deno" && \
    curl -fsSL https://deno.land/x/install/install.sh | DENO_INSTALL=/usr/bin sh

COPY . /build
WORKDIR /build
RUN cargo xtask build

FROM debian

COPY --from=builder /build/target/release/semantic /usr/bin/semantic

ENV SEMANTIC_ADDRESS=0.0.0.0:80

ENV SEMANTIC_DATA_PATH=/semantic
VOLUME /semantic

ENTRYPOINT [ "/usr/bin/semantic" ]
