FROM rust:1 AS build
WORKDIR /src
COPY . .
# The query! macros are checked against .sqlx/, so the build needs no database.
ENV SQLX_OFFLINE=true
RUN cargo build --release -p server

FROM debian:stable-slim
COPY --from=build /src/target/release/learnly-server /usr/local/bin/learnly-server
CMD ["learnly-server", "serve"]
