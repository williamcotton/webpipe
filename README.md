![Test Suite](https://github.com/williamcotton/webpipe-rs/workflows/Test%20Suite/badge.svg)

<img src="./wp.png" width="200">

## Web Pipe

WebPipe is a small HTTP server and test runner powered by a compact DSL. You write routes and tests in a single `.wp` file, then run a server that executes the pipelines. Here is a minimal hello world you can paste into a file like `hello.wp`:

```text
GET /hello/:world
  |> jq: `{ world: .params.world }`
  |> handlebars: `<p>hello, {{world}}</p>`

describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and output equals `<p>hello, world</p>`
```

Run the server with cargo, pointing to your `.wp` file. The server listens on `127.0.0.1:8090` by default, or you can pass a `host:port` after the file path. Try opening `http://127.0.0.1:8090/hello/world`.

```bash
cargo run -- hello.wp
# or choose an address
cargo run -- hello.wp 0.0.0.0:8090
```

Run tests defined inside the same `.wp` file by passing `--test`. The process exits nonzero on failure and prints a concise summary.

```bash
cargo run -- hello.wp --test
```

During development the server watches your `.wp` file and restarts on changes. It also loads `.env` and `.env.local` from the same directory as the `.wp` file so you can provide configuration without changing code.

For a larger example and more middleware like `pg`, `fetch`, `lua`, `cache`, `auth`, `log`, and `debug`, see `comprehensive_test.wp`. Build with `cargo build --release` to produce an optimized binary at `target/release/webpipe`.

