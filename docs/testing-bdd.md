### Testing & BDD

- **Describe/It** blocks live in the same `.wp` file.
- Structure:
  - `describe "name"` groups tests and can declare shared mocks.
  - `it "does thing"` defines a single test case.
  - You can add test-level mocks that override describe-level mocks.

- Kinds of tests:
  - Route calls: `when calling GET /path?query`
  - Pipeline execution: `when executing pipeline getTeams`
  - Variable execution: `when executing variable <middleware> <name>`

- Providing input:
  - For `executing pipeline` and `executing variable`, you can attach input JSON:
    - `with input `{ "params": { "id": 2 } }``
  - Route calls build inputs from the method/path (including `?query`); body/headers are not injected in tests.

- Mocks:
  - Describe-level: `with mock <middleware> returning `<json>``
  - Variable-level: `with mock <middleware>.<variable> returning `<json>``
  - Pipeline-level: `with mock pipeline <name> returning `<json>``
  - Test-level override: inside an `it` block use `and mock ...` to override.
  - If a step uses `resultName: "X"`, a mock can target `<middleware>.X`.

Assertions:
- **Status**: `then status is 200`, `then status in 200..299`
- **Content-Type**: `then contentType is "text/html"`
- **Output**:
  - `then output equals `{"a":1}`
  - `then output contains `{"a":1}`
  - `then output matches `^<p>hello, .*</p>$``
  - With jq path: `then output `.rows[0].id` equals 42`

Run tests:
```bash
cargo run app.wp --test
```

#### Examples

- Executing a variable with a variable-specific mock:
```wp
describe "teamsQuery variable"
  with mock pg.teamsQuery returning `{
    "rows": [
      { "id": "1", "name": "Platform" }
    ]
  }`

  it "returns all teams"
    when executing variable pg teamsQuery
    with input `{ "sqlParams": [] }`
    then output equals `{
      "rows": [
        { "id": "1", "name": "Platform" }
      ]
    }`
```

- Executing a pipeline with describe-level and test-level mocks; asserting jq on a subpath:
```wp
describe "getTeams pipeline"
  with mock pg returning `{
    "rows": [{ "id": "2", "name": "Growth" }]
  }`

  it "transforms params and queries database"
    when executing pipeline getTeams
    with input `{ "params": { "id": "2" } }`
    then output equals `{
      "rows": [{ "id": "2", "name": "Growth" }]
    }`

  it "supports jq equals on output subpath"
    when executing pipeline getTeams
    with input `{ "params": { "id": 42 } }`
    and mock pg returning `{ "rows": [{ "id": 42, "name": "Marketing" }] }`
    then output `.rows[0].id` equals 42
```

- Using a jq filter with map to assert array transformation:
```wp
describe "jq map assertions"
  it "supports jq filter with map"
    when executing pipeline getTeams
    with input `{ "params": { "id": 2 } }`
    and mock pg returning `{ "rows": [{ "id": 2, "name": "Growth" }, { "id": 3, "name": "Security" }] }`
    then output `.rows | map(.id)` equals `[2, 3]`
```

- Calling a route and asserting status, contains, regex match, and range:
```wp
describe "jq assertions"
  it "supports output contains for partial JSON"
    when calling GET /hello
    then status is 200
    and output contains `{ "hello": "world" }`

  it "supports output matches for HTML"
    when calling GET /hello/world
    then output matches `^<p>hello, .*</p>$`

  it "supports status ranges"
    when calling GET /hello
    then status in 200..299
```

Notes:
- `equals` compares full JSON values; `contains` does deep partial matching for objects and arrays; `matches` uses a regex on the stringified value.
- For `output` assertions with a jq path (backticked after `output`), the filter is applied to the output first, then the comparison is evaluated.


