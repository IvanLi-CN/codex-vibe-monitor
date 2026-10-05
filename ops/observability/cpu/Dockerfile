# One-shot sampler only. Supply the verified upstream samply 0.13.1 binary for
# this platform in the build context; operators bind its immutable local image ID.
FROM debian:bookworm-slim
COPY samply /usr/local/bin/samply
RUN chmod 0755 /usr/local/bin/samply
USER 0
ENTRYPOINT ["/bin/sh", "-c", "umask 077; exec /usr/local/bin/samply \"$@\"", "samply"]
