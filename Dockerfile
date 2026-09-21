FROM rust:1.85-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p jev-quantum-server

FROM debian:bookworm-slim
RUN useradd --system --home /app --shell /usr/sbin/nologin quantum
COPY --from=build /src/target/release/jev-quantum-server /usr/local/bin/jev-quantum-server
USER quantum
EXPOSE 3000
ENTRYPOINT ["jev-quantum-server"]
CMD ["--bind", "0.0.0.0:3000"]
