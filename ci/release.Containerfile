# The release artifact is compiled inside this one immutable x86_64 environment.
# The platform manifest digest pins the complete upstream Rust image rather than
# a moving tag. The tag is retained as a review aid and is checked against
# rust-toolchain.toml below.
FROM ghcr.io/rust-lang/rust:1.99.1-slim-bookworm@sha256:220ee93216b2acf3c04f312e24bf5d155c8eed61d707fc1ff8553ae24609da4e

# A fixed Debian snapshot makes package resolution immutable too. In particular,
# vendored libgit2 and zlib compile with this image's C compiler, linker, archive
# tool and musl headers instead of whichever versions the caller happens to run.
ARG DEBIAN_SNAPSHOT=20261101T000000Z
RUN rm -f /etc/apt/sources.list.d/* \
    && printf '%s\n' \
       "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/ bookworm main" \
       "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/ bookworm-security main" \
       > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       git \
       musl-tools \
       pkg-config \
       zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml /tmp/aplan-rust-toolchain.toml
COPY ci/scripts/toolchain.sh /tmp/aplan-toolchain.sh

# The image digest already pins Rust, but comparing it with the repository's
# single toolchain contract makes a Rust bump fail until the builder pin moves in
# the same reviewed change. The musl target is installed into the pinned image,
# never into the caller's host toolchain.
RUN chmod +x /tmp/aplan-toolchain.sh \
    && required="$(/tmp/aplan-toolchain.sh channel /tmp/aplan-rust-toolchain.toml)" \
    && actual="$(rustc --version | awk '{print $2}')" \
    && test "$actual" = "$required" \
    && rustup target add x86_64-unknown-linux-musl --toolchain "$required" \
    && rm /tmp/aplan-rust-toolchain.toml /tmp/aplan-toolchain.sh

WORKDIR /aplan/source
