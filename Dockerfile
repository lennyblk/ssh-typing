FROM rust:1-slim-bookworm AS build
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev
# Dépendances d'abord : recompilées seulement si Cargo.* change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

FROM debian:bookworm-slim
LABEL app=ssh-typing
RUN useradd -u 10001 -m game && install -d -o game /data
COPY --from=build /app/target/release/ssh-typing /usr/local/bin/
USER game
ENV LISTEN=0.0.0.0:2222 HOST_KEY=/data/host_key RECORDS=/data/records.txt ACCOUNTS=/data/accounts.json
VOLUME /data
EXPOSE 2222
CMD ["ssh-typing"]
