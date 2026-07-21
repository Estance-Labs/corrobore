# syntax=docker/dockerfile:1

# The Rust service is built from committed lockfiles. The runtime remains
# distroless: glibc + the binary only (no shell or package manager).

# ---- Healthcheck utility stage --------------------------------------------
# The musl variant is static, so the single BusyBox binary remains usable in
# the shell-free distroless runtime image.
FROM busybox:1.37.0-musl AS healthcheck

# ---- Rust build stage ------------------------------------------------------
FROM rust:1.96.0-slim AS builder

WORKDIR /build

# Copy the full workspace so path dependencies resolve, then build only the
# HTTP server binary with the committed lockfile for reproducibility.
COPY . .
RUN cargo build --release --locked -p corrobore-http-server

# Prepare a writable runtime data directory owned by the distroless nonroot
# user (uid 65532); distroless has no shell to `mkdir` at runtime.
RUN mkdir -p /data && chown 65532:65532 /data

# ---- Runtime stage ---------------------------------------------------------
# distroless/cc provides glibc + libgcc for the dynamically linked binary.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=builder /build/target/release/corrobore-http-server /usr/local/bin/corrobore-http-server
COPY --from=healthcheck /bin/busybox /usr/local/bin/busybox
COPY --from=builder --chown=65532:65532 /data /data

# Bind on all interfaces inside the container and persist runtime state to the
# mounted volume. CORROBORE_HTTP_AUTH_TOKEN must be supplied at run time.
ENV CORROBORE_HTTP_HOST=0.0.0.0 \
    CORROBORE_HTTP_PORT=8080 \
    CORROBORE_HTTP_SESSION_STORE_DIR=/data

USER nonroot
EXPOSE 8080
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/corrobore-http-server"]
