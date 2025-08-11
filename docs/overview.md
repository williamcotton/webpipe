### Overview

Web Pipe (wp) is a small DSL and runtime for composing HTTP endpoints from middleware steps that transform JSON.

- **Core ideas**: routes, pipelines, middleware, variables, configs, result routing, testing.
- **Execution model**: each step receives JSON, returns JSON or a string (e.g., HTML). Steps can attach metadata (e.g., cache/log) that later steps respect.
- **Built-in middleware**: jq, pg, fetch, handlebars, lua, auth, cache, log, debug, validate.

Minimal example:

```wp
GET /hello/:name
  |> jq: `{ name: .params.name }`
  |> handlebars: `<p>hello, {{name}}</p>`
```


