# syntax=docker/dockerfile:1

# The Rust service is built from committed lockfiles. The runtime remains
# distroless: glibc + the binary only (no shell or package manager).

# ---- Rust build stage ------------------------------------------------------
FROM rust:1.96.0-slim-bookworm AS builder

WORKDIR /build

ARG CORROBORE_BUILD_VERSION=0.2.2
ARG CORROBORE_BUILD_REVISION=unknown

# Copy the full workspace so path dependencies resolve, then build the unified
# standalone product with the committed lockfile and reproducible revision.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    CORROBORE_BUILD_REVISION="${CORROBORE_BUILD_REVISION}" \
    cargo build --release --locked -p corrobore-http-server --bin corrobore && \
    cp target/release/corrobore /build/corrobore-dist

# Prepare a writable runtime data directory owned by the distroless nonroot
# user (uid 65532); distroless has no shell to `mkdir` at runtime.
RUN mkdir -p /data /etc/corrobore && chown -R 65532:65532 /data /etc/corrobore

# ---- Runtime stage ---------------------------------------------------------
# distroless/cc provides glibc + libgcc for the dynamically linked binary.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

ARG CORROBORE_BUILD_VERSION=0.2.2
ARG CORROBORE_BUILD_REVISION=unknown

LABEL org.opencontainers.image.title="corrobore" \
      org.opencontainers.image.description="Corrobore standalone graph server" \
      org.opencontainers.image.version="${CORROBORE_BUILD_VERSION}" \
      org.opencontainers.image.revision="${CORROBORE_BUILD_REVISION}" \
      org.opencontainers.image.source="https://github.com/Noetance-Labs/corrobore"

WORKDIR /app

COPY --from=builder /build/corrobore-dist /usr/local/bin/corrobore
COPY --from=builder --chown=65532:65532 /data /data
COPY --chown=65532:65532 packaging/corrobore.production.toml /etc/corrobore/corrobore.toml

USER 65532:65532
EXPOSE 8080
VOLUME ["/data"]
STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=5 \
  CMD ["/usr/local/bin/corrobore", "server", "status", "--config", "/etc/corrobore/corrobore.toml"]

ENTRYPOINT ["/usr/local/bin/corrobore"]
CMD ["server", "start", "--config", "/etc/corrobore/corrobore.toml"]
