# Web Pipe Documentation

A concise index for everything Web Pipe—from first steps to advanced usage.

## Core Guides

1. **Overview** — What Web Pipe is and why it exists.  
   [overview.md](./overview.md)

2. **Getting Started** — Install, run, and create your first pipeline.  
   [getting-started.md](./getting-started.md)

3. **DSL Syntax** — The Web Pipe language: control flow, inline arguments, and blocks.  
   [dsl-syntax.md](./dsl-syntax.md)

4. **GraphQL** — Native support for schemas, resolvers, and queries.  
   [graphql.md](./graphql.md)

## Reference

- **Configuration** — Defining `config` blocks, environment variables, and secrets.  
  [config.md](./config.md)

- **Routes & Pipelines** — Declaring routes, composing steps, and result tagging.  
  [routes-and-pipelines.md](./routes-and-pipelines.md)

- **Inline Arguments** — Passing dynamic data directly to middleware.  
  [inline-arguments.md](./inline-arguments.md)

- **Context & Metadata** — Accessing system state (`$context`) in middleware.  
  [context.md](./context.md)

- **Concurrency** — Async tasks and joining results.  
  [concurrency.md](./concurrency.md)

- **Flow Control** — Conditional execution with `if`, `dispatch`, `foreach` and tags (`@when`, `@guard`).
  [flow-control.md](./flow-control.md)

- **Variables & Partials** — Reusable SQL, templates, and pipeline fragments.  
  [variables-and-partials.md](./variables-and-partials.md)

- **Result Routing** — Setting status codes, content types, and response shaping.  
  [result-routing.md](./result-routing.md)

- **Error Handling** — Fail-fast vs. recoverable errors, custom error outputs.  
  [error-handling.md](./error-handling.md)

## Quality & Testing

- **Mocking** — Stubbing middleware and deterministic test inputs.  
  [mocking.md](./mocking.md)

- **Testing & BDD** — Spec syntax, `let` variables, DOM assertions, and end-to-end tests.  
  [testing-bdd.md](./testing-bdd.md)

## Middleware Guides

- **jq** — JSON transformation and shaping within pipelines.  
  [middleware/jq.md](./middleware/jq.md)

- **pg** — PostgreSQL queries and parameter binding.  
  [middleware/pg.md](./middleware/pg.md)

- **fetch** — HTTP calls to external services.  
  [middleware/fetch.md](./middleware/fetch.md)

- **handlebars** — Server-side templating for HTML/text.  
  [middleware/handlebars.md](./middleware/handlebars.md)

- **lua** — Embed custom logic and scripting.  
  [middleware/lua.md](./middleware/lua.md)

- **auth** — Authentication helpers and guards.  
  [middleware/auth.md](./middleware/auth.md)

- **cache** — Response and query caching strategies.  
  [middleware/cache.md](./middleware/cache.md)

- **rateLimit** — Token bucket rate limiting.  
  [middleware/rate_limit.md](./middleware/rate_limit.md)

- **join** — Synchronize async tasks.  
  [middleware/join.md](./middleware/join.md)

- **log** — Structured request/response logging.  
  [middleware/log.md](./middleware/log.md)

- **debug** — Introspection tools for development.  
  [middleware/debug.md](./middleware/debug.md)

- **validate** — Input validation and schema checks.  
  [middleware/validate.md](./middleware/validate.md)

## Operations

- **Database Setup** — Local and CI database configuration.  
  [database-setup.md](./database-setup.md)

- **Docker & Deployment** — Containerization, images, and runtime environments.  
  [deployment-docker.md](./deployment-docker.md)