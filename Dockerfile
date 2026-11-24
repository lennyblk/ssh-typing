FROM rust:1-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev && cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd -u 10001 -m game && install -d -o game /data
COPY --from=build /app/target/release/ssh-typing /usr/local/bin/
USER game
ENV LISTEN=0.0.0.0:2222 HOST_KEY=/data/host_key
VOLUME /data
EXPOSE 2222
CMD ["ssh-typing"]
