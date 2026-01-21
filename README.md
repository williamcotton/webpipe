![Test Suite](https://github.com/williamcotton/webpipe/workflows/CI/badge.svg)

<img src="./wp.png" width="200">

## Web Pipe

Web Pipe is an **experimental language and runtime** for building web applications using **pipeline-oriented request processing**.

![debugger](https://github.com/user-attachments/assets/a8a803b6-a6ff-48c3-954e-bb838bb19cd3)

Instead of separating routing, middleware, database access, templating, GraphQL, and testing into distinct frameworks or layers, Web Pipe treats them as variations of the same thing: a request flowing through a sequence of transformations over explicit JSON data. The intent is not to compete with existing web frameworks, but to explore what happens when a single execution model is applied consistently across concerns that are usually siloed.

This repository represents an experiment in language and systems design. Stability, familiarity, and completeness are explicitly not the goal. The goal is the journey of discovery and not the destination of a popular programming language.



Here is a minimal hello world you can paste into a file like `hello.wp`:

```wp
GET /hello/:world
  |> jq: `{ world: .params.world }`
  |> handlebars: `<p>hello, {{world}}</p>`

describe "hello, world"
  it "calls the route"
    let world = "world"
    
    when calling GET /hello/{{world}}
    then status is 200
    and selector `p` text equals "hello, {{world}}"
```

Run the server with cargo, pointing to your `.wp` file. The server listens on `127.0.0.1:7770` by default. Try opening `http://127.0.0.1:7770/hello/world`.

```bash
cargo run hello.wp
```

Run tests defined inside the same `.wp` file by passing `--test`. The process exits nonzero on failure and prints a concise summary.

```bash
cargo run hello.wp --test
```

You can also try it out with Docker.

```Dockerfile
FROM ghcr.io/williamcotton/webpipe:latest

COPY app.wp /app/
COPY public /app/public/
COPY scripts /app/scripts/

CMD ["app.wp"]
```

During development the server watches your `.wp` file and restarts on changes. It also loads `.env` and `.env.local` from the same directory as the `.wp` file so you can provide configuration without changing code.

For a tutorial example and more middleware like `pg`, `fetch`, `lua`, `cache`, `auth`, `log`, and `debug`, see `example.wp`. Build with `cargo build --release` to produce an optimized binary at `target/release/webpipe`.

### Documentation

See the full documentation index at [docs/README.md](docs/README.md).
