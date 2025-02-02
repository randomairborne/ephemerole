FROM rust:alpine AS builder

RUN apk add musl-dev

WORKDIR /build

COPY . .

RUN cargo build --release

FROM scratch

COPY --from=builder /build/target/release/ephemerole /usr/bin/ephemerole

ENTRYPOINT ["/usr/bin/ephemerole"]
