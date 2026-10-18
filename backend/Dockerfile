FROM rust:1 AS build
WORKDIR /src
COPY . .
# The query! macros are checked against .sqlx/, so the build needs no database.
ENV SQLX_OFFLINE=true
RUN cargo build --release -p server

FROM debian:stable-slim
COPY --from=build /src/target/release/vumlo-server /usr/local/bin/vumlo-server
CMD ["vumlo-server", "serve"]
