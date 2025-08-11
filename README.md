![Test Suite](https://github.com/williamcotton/webpipe-rs/workflows/CI/badge.svg)

<img src="./wp.png" width="200">

## Web Pipe

Web Pipe (wp) is an **experimental** DSL and runtime for building web APIs and applications through pipeline-based request processing. Each HTTP request flows through a series of middleware that transform JSON data, enabling composition of data processing steps.

Here is a minimal hello world you can paste into a file like `hello.wp`:

```wp
GET /hello/:world
  |> jq: `{ world: .params.world }`
  |> handlebars: `<p>hello, {{world}}</p>`

describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and output equals `<p>hello, world</p>`
```

Run the server with cargo, pointing to your `.wp` file. The server listens on `127.0.0.1:8090` by default. Try opening `http://127.0.0.1:8090/hello/world`.

```bash
cargo run hello.wp
```

Run tests defined inside the same `.wp` file by passing `--test`. The process exits nonzero on failure and prints a concise summary.

```bash
cargo run hello.wp --test
```

You can also try it out with Docker.

```Dockerfile
FROM ghcr.io/williamcotton/webpipe-rs:latest

COPY app.wp /app/
COPY public /app/public/
COPY scripts /app/scripts/

CMD ["app.wp"]
```

During development the server watches your `.wp` file and restarts on changes. It also loads `.env` and `.env.local` from the same directory as the `.wp` file so you can provide configuration without changing code.

For a tutorial example and more middleware like `pg`, `fetch`, `lua`, `cache`, `auth`, `log`, and `debug`, see `example.wp`. Build with `cargo build --release` to produce an optimized binary at `target/release/webpipe`.
