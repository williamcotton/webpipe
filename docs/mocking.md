### Mocking

- **Middleware-level**: `with mock pg returning `{"rows":[]}`
- **Variable-level**: `with mock pg.teamsQuery returning `{"rows":[...]}`
- **Pipeline-level**: `with mock pipeline getTeams returning `{"rows":[...]}`

When `resultName` is set before a mocked middleware, the mock can be targeted with `<middleware>.<resultName>`.


