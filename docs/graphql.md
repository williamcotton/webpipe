# Native GraphQL Support

WebPipe 2.0 introduces a fully integrated GraphQL runtime, allowing you to define schemas, map resolvers to pipelines, and execute queries efficiently.

## Defining a Schema

Use the `graphqlSchema` block in your `.wp` files to define your GraphQL schema using standard Schema Definition Language (SDL).

```graphql
graphqlSchema = `
  type User {
    id: ID!
    name: String!
    email: String!
  }

  type Query {
    users: [User!]!
    user(id: ID!): User
  }

  type Mutation {
    createUser(name: String!, email: String!): User!
  }
`
```

## Mapping Resolvers

Resolvers are mapped to WebPipe pipelines using the `query` and `mutation` keywords. Each resolver pipeline receives the GraphQL arguments and parent object in its input.

### Query Resolvers

```wp
# Resolver for Query.users
query users =
  |> pg: `SELECT * FROM users`
  |> jq: `.data.rows`

# Resolver for Query.user(id: ID!)
query user =
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `.data.rows[0]`
```

### Mutation Resolvers

```wp
# Resolver for Mutation.createUser
mutation createUser =
  |> auth: "required"
  |> jq: `{ sqlParams: [.name, .email] }`
  |> pg: `INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *`
  |> jq: `.data.rows[0]`
```

## Execution Pipeline

To expose your GraphQL API, use the `graphql` middleware in a route. This middleware executes the query against your defined schema and resolvers.

```wp
POST /graphql
  |> auth: "optional"
  |> graphql: `query` # The config here is usually dynamic or passed from input
```

Typically, you'll want to accept the query from the HTTP request body:

```wp
POST /graphql
  |> auth: "optional"
  # Extract query and variables from request body
  |> jq: `.graphqlParams = .body` 
  # Execute GraphQL
  |> graphql: .body.query
```

*(Note: The actual `graphql` middleware implementation handles the execution details. The example above simplifies the common pattern.)*

## Advanced Usage

### resultName Pattern

You can use the `resultName` pattern to execute multiple GraphQL queries in a single pipeline and aggregate the results.

```wp
GET /dashboard
  |> jq: `.resultName = "todos"`
  |> graphql: `query { todos { id title } }`
  |> jq: `.resultName = "user"`
  |> graphql: `query { currentUser { name email } }`
  |> jq: `{ todos: .data.todos.data.todos, user: .data.user.data.currentUser }`
```

