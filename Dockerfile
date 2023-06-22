FROM rust
COPY . ./app
# set working directory
WORKDIR /app

RUN cargo b -r
CMD ["/app/target/release/mempool-server"]
