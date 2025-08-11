### DSL Syntax

- **Config block**:
```wp
config pg {
  host: $WP_PG_HOST || "localhost"
}
```

- **Variables** (named snippets):
```wp
pg teamsQuery = `SELECT * FROM teams`
handlebars header = `<h1>{{title}}</h1>`
```

- **Pipelines**:
```wp
pipeline getTeams =
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: teamsQuery
```

- **Routes**:
```wp
GET /teams/:id
  |> pipeline: getTeams
  |> jq: `{ team: .data.teamsQuery.rows[0] }`
```

- **Tests**: see `testing-bdd.md`.


