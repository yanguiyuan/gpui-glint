ARG RUST_IMAGE=rust:1.95-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim
FROM ${RUST_IMAGE} AS build
ARG RUST_TOOLCHAIN=1.95.0
ENV RUSTUP_TOOLCHAIN=${RUST_TOOLCHAIN}
WORKDIR /workspace
COPY . .
RUN cargo build --release --locked -p glint-plugin-server -p glint-plugin-cli

FROM ${RUNTIME_IMAGE}
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN useradd --system --uid 10001 --create-home glint \
    && mkdir /data && chown glint:glint /data
COPY --from=build /workspace/target/release/glint-plugin-server /usr/local/bin/
COPY --from=build /workspace/target/release/glint-pluginctl /usr/local/bin/
USER 10001:10001
ENV GLINT_REGISTRY_DATA=/data GLINT_REGISTRY_LISTEN=0.0.0.0:8787
EXPOSE 8787
VOLUME ["/data"]
ENTRYPOINT ["glint-plugin-server"]
