# Web Pipeline (wp)

You are to make a web server runtime for the following language:

```wp
GET /page/:id
  |> jq: `{ id: .params.id }`
  |> lua: `return { sqlParams: { request.id } }`
  |> pg: `select * from items where id = $1`

pg articlesQuery = `select * from articles`

GET /articles
  |> pg: articlesQuery
```

Each step in the pipeline has a plugin. In the above example there are three plugins, jq, lua, and pg.

Use a per-request bump allocator memory arena for the jansson custom allocators as well as any other memory allocation needed. Release the memory arena after the end of each request is handled.

Use libmicrohttpd to create event handlers that parse an income HTTP request into a jansson json_t type. This type is passed between plugins. In essence each step in the pipeline takes in json_t and returns json_t. The plugin should also take in the memory arena.

The initial JSON that is created for the first step in the pipeline is a request object with keys for query, body, params, headers, and the rest of what a request object in ExpressJS would look like. The intial request is maintained across each step of JSON passed between steps in a pipeline.

In the above example we can see that jq step with `{ id: .params.id }` - the jq plugin is called with the intial request object. It sets the id key on the request object to the id param from the URL. Then the next step in the pipeline is the lua interpreter which has a request in scope as `return { sqlParams: request.id }`. This prepares the sqlParams, a required keyword for the pg plugin, which is the next step in the pipeline. The pg plugin takes in the json_t object with the sqlParams array and uses that with the postgres query. Then it returns a data key on the request object with a rows array for the query results.

Each plugin should register its callback and name with a central location. Each plugin should be a separate .so file and dynamically loaded by the main wp process. The plugin .so files to be dynamically loaded come from the AST. They take in a at least a json_t object and the per-request memory arena.

For the jq plugin you will need a janssonToJv as well as a jvToJansson. You should figure out a way to cache the compiled jq at startup so the requests are more performant.

The same applies for lua, you'll need to create a janssonToLua as well as a luaToJansson.

The same goes for pg, the postgres plugin.

This way each step in the pipeline can process JSON data.

```wp
POST /api/employees {
  |> validate: {
    name: string(10..100)
    email: email
    team_id?: number
  }
  |> lua: `
    return {sqlParams = {body.name, body.email, body.team_id}}
  `
  |> pg: insertEmployee
  |> jq: `{
    success: true,
    employee: .data[0].rows[0]
  }`
  |> result
    ok(201):
      |> jq: `{
        success: true,
        data: .employee,
        meta: {
          timestamp: now,
          version: "1.0"
        }
      }`
    validationError(400):
      |> jq: `{
        error: "Validation failed",
        fields: .errors[0].field,
        rule: .errors[0].rule,
        meta: {
          timestamp: now,
          support: "help@example.com"
        }
      }`
    sqlError(500):
      |> jq: `{
        error: "Database error",
        sqlstate: .errors[0].sqlstate,
        message: .errors[0].message,
        meta: {
          timestamp: now,
          contact: "support@example.com"
        }
      }`
    default(500):
      |> jq: `{
        error: "Internal server error",
        meta: {
          timestamp: now,
          contact: "support@example.com"
        }
      }`
}
```

Built in to the language and runtime is the "result" step in a pipeline. You can see from the above example how it would work. Think like in a functional pipeline.

Your job is to write all of this in C. You can and should organize the files and build out the Makefile for being built with clang as the compiler. It only needs to build for clang on macOS.