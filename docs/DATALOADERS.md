# DataLoaders & Nested Resolvers in WebPipe 2.0

This document explains how to use DataLoaders and nested resolvers to solve the N+1 query problem in GraphQL.

## Table of Contents
- [What is the N+1 Problem?](#what-is-the-n1-problem)
- [How DataLoaders Solve It](#how-dataloaders-solve-it)
- [Syntax & Usage](#syntax--usage)
- [Database Example](#database-example)
- [Testing](#testing)

## What is the N+1 Problem?

The N+1 problem occurs when fetching nested data results in excessive database queries:

```graphql
query {
  teams {           # 1 query to get all teams
    id
    name
    employees {     # N queries (one per team) to get employees
      id
      name
    }
  }
}
```

If you have 10 teams, this results in **11 queries** (1 for teams + 10 for employees).

## How DataLoaders Solve It

DataLoaders batch multiple requests into a single query:

1. GraphQL resolver requests employees for Team 1
2. GraphQL resolver requests employees for Team 2
3. GraphQL resolver requests employees for Team 3
4. **DataLoader batches these into ONE request**: `EmployeesByTeamLoader` with `keys: [1, 2, 3]`
5. Single database query: `SELECT ... WHERE team_id IN (1, 2, 3)`

Result: **2 queries total** (1 for teams + 1 batched for all employees)

## Syntax & Usage

### 1. Define a Loader Pipeline

A loader pipeline accepts an object with a `keys` array and returns a map of results:

```webpipe
pipeline UserLoader =
  |> jq: `.keys as $userIds | { sqlParams: [$userIds] }`
  |> pg: `
    SELECT
      id,
      json_agg(
        json_build_object('id', id, 'name', name)
      ) as data
    FROM users
    WHERE id = ANY($1::int[])
    GROUP BY id
  `
  |> jq: `
    .data.rows | reduce .[] as $row (
      {};
      . + { ($row.id | tostring): $row.data }
    )
  `
```

**Input Format:**
```json
{
  "keys": [101, 102, 103]
}
```

**Output Format (must be a map keyed by the input keys):**
```json
{
  "101": {"id": 101, "name": "Alice"},
  "102": {"id": 102, "name": "Bob"},
  "103": {"id": 103, "name": "Charlie"}
}
```

### 2. Define a Nested Resolver

Use the `resolver` keyword to define how to resolve a nested field:

```webpipe
resolver Post.author =
  |> loader(.parent.userId): UserLoader
```

**Syntax:**
- `resolver TypeName.fieldName` - The GraphQL type and field
- `.parent.userId` - JQ expression to extract the key from the parent object
- `UserLoader` - The loader pipeline name

### 3. Use in GraphQL Queries

```graphql
query {
  posts {
    id
    title
    author {      # Uses DataLoader batching!
      id
      name
    }
  }
}
```

## Database Example

Here's a complete example using PostgreSQL:

### Schema Definition

```webpipe
graphqlSchema = `
  type Team {
    id: ID!
    name: String!
    employees: [Employee!]!
  }

  type Employee {
    id: ID!
    name: String!
    email: String!
    teamId: ID!
  }

  type Query {
    teams: [Team!]!
  }
`
```

### Query Resolver (Root)

```webpipe
query teams =
  |> pg: `SELECT id, name FROM teams`
  |> jq: `.data.rows`
```

### Loader Pipeline

```webpipe
pipeline EmployeesByTeamLoader =
  |> jq: `.keys as $teamIds | { sqlParams: [$teamIds] }`
  |> pg: `
    SELECT
      team_id,
      json_agg(
        json_build_object(
          'id', id::text,
          'teamId', team_id::text,
          'name', name,
          'email', email
        )
      ) as employees
    FROM employees
    WHERE team_id = ANY($1::int[])
    GROUP BY team_id
  `
  |> jq: `
    .data.rows | reduce .[] as $row (
      {};
      . + { ($row.team_id | tostring): $row.employees }
    )
  `
```

### Nested Resolver

```webpipe
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

### Query Example

```graphql
query {
  teams {
    id
    name
    employees {    # Batched! Only 1 query for all employees
      id
      name
      email
    }
  }
}
```

## Testing

### Unit Test Example

See `tests/loader_test.wp` for a complete working example:

```webpipe
describe "DataLoader N+1 Prevention"
  it "batches user lookups when resolving posts"
    when executing pipeline testPostsWithAuthors
    with input `{}`
    then output `.data.posts[0].author.name` equals "Alice"
    and output `.data.posts[1].author.name` equals "Bob"
    and output `.data.posts[2].author.name` equals "Alice"
```

### Integration Test

See `comprehensive_test.wp` for database-backed examples:

```webpipe
describe "DataLoader N+1 Prevention with Database"
  it "fetches teams with employees using DataLoader batching"
    when calling GET /test-teams-with-employees
    then status is 200
    and output `.data.teams` is array
    and output `.data.teams[0].employees` is array
```

## Implementation Details

### How Batching Works

1. **GraphQL Request:** Client requests teams with employees
2. **Root Resolver:** `query teams` executes and returns teams
3. **Nested Resolver:** For each team, `resolver Team.employees` is called
4. **DataLoader Batching:**
   - Collects all team IDs: `[1, 2, 3, 4, 5]`
   - Calls `EmployeesByTeamLoader` ONCE with `{ "keys": [1, 2, 3, 4, 5] }`
5. **Database Query:** Single `WHERE team_id = ANY($1)` query
6. **Result Distribution:** DataLoader maps results back to each team

### Performance Benefits

| Scenario | Without DataLoader | With DataLoader |
|----------|-------------------|-----------------|
| 10 teams | 11 queries | 2 queries |
| 100 teams | 101 queries | 2 queries |
| 1000 teams | 1001 queries | 2 queries |

### Key Features

✅ **Automatic Batching** - DataLoader collects all requests in a single tick
✅ **Caching** - Results are cached per-request
✅ **Type Safety** - Full GraphQL schema validation
✅ **Parent Context** - Access parent object via `.parent`
✅ **Inline Args** - Use JQ expressions to extract keys

## Advanced Patterns

### Multiple Loaders

```webpipe
# Loader for posts by user ID
pipeline PostsByUserLoader = ...

# Loader for comments by post ID
pipeline CommentsByPostLoader = ...

# Nested resolvers
resolver User.posts = |> loader(.parent.id): PostsByUserLoader
resolver Post.comments = |> loader(.parent.id): CommentsByPostLoader

# Query with 3 levels of nesting - still efficient!
query {
  users {
    posts {
      comments {
        text
      }
    }
  }
}
```

### Conditional Loading

```webpipe
resolver Post.author =
  |> jq: `if .parent.authorId then . else null end`
  |> loader(.parent.authorId): UserLoader
```

### Complex Keys

```webpipe
# Loader that uses compound keys
pipeline ItemsByCompositeKeyLoader =
  |> jq: `
    .keys | map(split(":")) as $pairs |
    { sqlParams: [$pairs] }
  `
  |> pg: `...`

# Resolver with composite key
resolver Order.item =
  |> jq: `.parent.itemType + ":" + .parent.itemId`
  |> loader(.): ItemsByCompositeKeyLoader
```

## Best Practices

1. **Always return a map** - Loader output must be keyed by input keys
2. **Handle missing keys** - Return `null` or empty array for missing items
3. **Convert IDs to strings** - JSON object keys are strings
4. **Use json_agg** - PostgreSQL's `json_agg` is perfect for batching
5. **Test with real data** - Use database tests to verify batching
6. **Monitor queries** - Use query logging to confirm N+1 is solved

## Troubleshooting

### Error: "Loader pipeline must return an object/map"

Your loader is returning an array instead of an object. Make sure to use `reduce` to convert rows to a map:

```webpipe
# ❌ Wrong - returns array
|> jq: `.data.rows`

# ✅ Correct - returns map
|> jq: `
  .data.rows | reduce .[] as $row (
    {};
    . + { ($row.id | tostring): $row }
  )
`
```

### Keys are not batching

Make sure you're using the `resolver` keyword, not just a regular `query`:

```webpipe
# ❌ Won't batch
query Post.author = ...

# ✅ Will batch
resolver Post.author = ...
```

### Parent value is undefined

Check that your GraphQL schema matches the resolver:

```webpipe
# Schema must define the field
type Post {
  author: User  # Must be defined here
}

# Then define the resolver
resolver Post.author = ...
```

## See Also

- [GraphQL DataLoader Specification](https://github.com/graphql/dataloader)
- [WebPipe GraphQL Guide](./GRAPHQL.md)
- [Inline Arguments RFC](./INLINE_ARGS.md)
