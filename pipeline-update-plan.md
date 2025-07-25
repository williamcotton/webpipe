# Pipeline Named Query Handling Plan

## Current Issue

The current pipeline implementation has a key problem with multiple database queries:

**Multiple Query Conflicts**: When multiple database queries are executed, each overwrites the previous `data` object, losing previous query results.

### Example Problem
```wp
pipeline getTeams =
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: `SELECT * FROM todos WHERE id = $1`

GET /page/:id
  |> pipeline: getTeams
```

**Current Response (loses request context and query names):**
```json
{
  "sqlParams": ["1"],
  "data": {
    "rows": [],
    "rowCount": 0
  }
}
```

**Desired Response (with named queries and preserved context):**
```json
{
  "method": "GET",
  "path": "/page/1", 
  "params": { "id": "1" },
  "query": {},
  "headers": {...},
  "body": null,
  "data": {
    "rows": [],
    "rowCount": 0
  }
}
```

## Proposed Solution: resultName Approach

### Core Concept
- Use `resultName` field in jq output to name middleware results (generic, not SQL-specific)
- Store each middleware result under its named key in the `data` object
- Preserve all request context throughout pipeline execution

### Syntax Design
```wp
pipeline getUser =
  |> jq: `{ sqlParams: [.params.id], resultName: "userProfile" }`
  |> pg: `SELECT * FROM users WHERE id = $1`

pipeline getUserTeams = 
  |> jq: `{ sqlParams: [.params.id], resultName: "userTeams" }`
  |> pg: `SELECT * FROM teams WHERE user_id = $1`

GET /user/:id/profile
  |> pipeline: getUser
  |> pipeline: getUserTeams
  |> jq: `{ 
      user: .data.userProfile.rows[0],
      teams: .data.userTeams.rows,
      teamCount: (.data.userTeams.rows | length)
    }`
```

**Expected Response:**
```json
{
  "method": "GET",
  "path": "/user/123/profile",
  "params": { "id": "123" },
  "query": {},
  "headers": {...},
  "body": null,
  "data": {
    "userProfile": {
      "rows": [{"id": 123, "name": "John", "email": "john@example.com"}],
      "rowCount": 1
    },
    "userTeams": { 
      "rows": [{"id": 1, "name": "Engineering"}, {"id": 2, "name": "Product"}],
      "rowCount": 2
    }
  },
  "user": {"id": 123, "name": "John", "email": "john@example.com"},
  "teams": [{"id": 1, "name": "Engineering"}, {"id": 2, "name": "Product"}],
  "teamCount": 2
}
```

### Data Structure Design

```json
{
  "data": {
    "queryName1": { "rows": [...], "rowCount": N },
    "queryName2": { "rows": [...], "rowCount": N },
    "queryName3": { "rows": [...], "rowCount": N }
  }
}
```

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1)

#### 1. Update All Database Middleware (Generic Approach)
- Check for optional `resultName` field in input JSON (works for any middleware)
- If using a variable (e.g., `|> pg: variableName`), automatically use variable name as result name
- If `resultName` is explicitly provided, use that instead
- If neither is present, use current behavior (store result directly in `data`)
- Preserve all other fields from input JSON

**Implementation Details (example for pg middleware):**
```c
// In any database middleware execution (pg, mongo, redis, etc.)
json_t *result_name = json_object_get(input, "resultName");
const char *variable_name = /* passed from pipeline execution for |> middleware: variableName */;

const char *data_key = NULL;
if (result_name && json_is_string(result_name)) {
    // Explicit resultName takes precedence
    data_key = json_string_value(result_name);
} else if (variable_name) {
    // Auto-use variable name
    data_key = variable_name;
}

if (data_key) {
    // Named result: data.keyName = result
    json_t *data_obj = json_object_get(input, "data");
    if (!data_obj) {
        data_obj = json_object();
        json_object_set(output, "data", data_obj);
    }
    json_object_set(data_obj, data_key, middleware_result);
} else {
    // No name provided: data = result (current behavior)
    json_object_set(output, "data", middleware_result);
}
```

#### 2. Update Pipeline Execution (`src/server.c`)
- Modify `execute_pipeline_with_result()` to preserve request context
- Automatically add `resultName` field when using variables (e.g., `|> pg: variableName`)
- Ensure request fields (method, path, params, etc.) are maintained
- Only allow middleware to add/modify their specific fields

**Key Changes:**
- Start with full request object as pipeline input
- Each middleware step adds to the object rather than replacing it
- When executing `|> middleware: variableName`, automatically add `resultName: "variableName"` to the JSON input (unless explicit `resultName` already exists)
- Remove the current complete replacement behavior

**Variable Name Auto-Addition:**
```c
// In execute_pipeline_with_result() when processing any middleware step with variable
if (step->variable_name && !json_object_get(middleware_input, "resultName")) {
    // Auto-add resultName for variable-based middleware calls
    json_object_set(middleware_input, "resultName", 
                   json_string(step->variable_name));
}
```

### Phase 2: Testing and Validation (Week 2)

## Testing Strategy

### Leveraging Existing Test Structure

#### 1. Unit Tests (`test/unit/`)
**Extend existing files with new test functions:**

**`test_pipeline.c`** - Add named query pipeline logic tests:
```c
void test_pipeline_sqlQueryName_processing()
void test_pipeline_pg_variable_auto_naming()  
void test_pipeline_request_context_preservation()
void test_pipeline_data_key_priority()
```

**`test_middleware.c`** - Add middleware naming behavior tests:
```c
void test_middleware_data_key_assignment()
void test_middleware_legacy_behavior_preserved()
void test_middleware_variable_name_passing()
```

#### 2. Integration Tests (`test/integration/`)
**Extend existing files:**

**`test_pg.c`** - Add pg middleware naming tests:
```c
void test_pg_with_resultName()
void test_pg_with_variable_auto_naming()
void test_pg_without_naming_legacy()
void test_pg_naming_priority_explicit_over_auto()
```

**`test_pipeline.c`** - Add pipeline integration tests:
```c
void test_multiple_named_queries_in_pipeline()
void test_mixed_naming_strategies()
void test_pipeline_data_accumulation()
void test_nested_pipelines_with_naming()
```

#### 3. Test Using Existing test.wp

**Modify existing `test.wp` routes** to test new functionality:
```wp
# Add to existing test.wp file

# Test pg variable auto-naming (modify existing getTeams pipeline)
pipeline getTeams =
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: `SELECT * FROM teams WHERE id = $1`

GET /page/:id
  |> pipeline: getTeams  # Should auto-store as data.getTeams or preserve existing behavior

# Test explicit resultName
GET /test/named-query/:id
  |> jq: `{ sqlParams: [.params.id], resultName: "userProfile" }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `{ user: .data.userProfile.rows[0] }`

# Test multiple named results
GET /test/multiple/:id
  |> jq: `{ sqlParams: [.params.id], resultName: "user" }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `{ sqlParams: [.params.id], resultName: "userTeams" }`
  |> pg: `SELECT * FROM teams WHERE user_id = $1`
  |> jq: `{ 
      profile: .data.user.rows[0],
      teams: .data.userTeams.rows
    }`
```

### Testing Approach

#### Test Execution
```bash
# Use existing test targets
make test-unit      # Run all unit tests (including new functions)
make test-integration  # Run all integration tests (including new functions)  

# Test with existing test.wp
./build/wp test.wp --test --timeout 10
# Run existing system tests to ensure compatibility

# Test for leaks
make test-leaks-unit
make test-leaks-integration
make test-leaks-system
```

#### Validation Strategy

**1. Backward Compatibility:**
- All existing tests must pass unchanged
- All existing `test.wp` endpoints work as before
- No regression in performance or functionality

**2. New Functionality (test via modified test.wp):**
- Test explicit `resultName` behavior
- Test variable auto-naming (works for any middleware)
- Test priority order (explicit > auto > legacy)
- Test request context preservation
- Test multiple named result scenarios

**3. Edge Cases:**
- Empty result sets with naming
- Invalid resultName values (null, empty string)
- Missing variables
- Duplicate result names in pipeline

### Success Criteria

#### Functional Requirements
- ✅ All existing tests pass without modification
- ✅ Named results store under correct keys
- ✅ Variable auto-naming works correctly (any middleware)
- ✅ Request context preserved throughout pipelines
- ✅ Legacy behavior unchanged for unnamed results

#### Performance Requirements  
- ✅ No performance regression > 5%
- ✅ Memory usage remains within existing bounds
- ✅ No memory leaks with named results

#### Test Coverage
- ✅ New code paths covered by unit tests
- ✅ Integration scenarios tested
- ✅ End-to-end workflows validated through test.wp modifications

#### 1. Update Test Cases
- Add test functions to existing test files as outlined above
- Test all naming strategies via test.wp modifications
- Validate backward compatibility thoroughly

#### 2. Update Example Application
```wp
# Update test.wp examples

# Using pg variables (auto-named)
pg getUserQuery = `SELECT * FROM users WHERE id = $1`
pg getTeamsQuery = `SELECT * FROM teams WHERE user_id = $1`

GET /user/:id/profile
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: getUserQuery  # Auto-stored as data.getUserQuery
  |> jq: `{ sqlParams: [.data.getUserQuery.rows[0].id] }`
  |> pg: getTeamsQuery  # Auto-stored as data.getTeamsQuery
  |> jq: `{ 
      user: .data.getUserQuery.rows[0],
      teams: .data.getTeamsQuery.rows
    }`

# Mixed approach (auto + explicit naming)
pg teamsQuery = `SELECT * FROM teams`

GET /dashboard/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: teamsQuery  # Auto-stored as data.teamsQuery
  |> jq: `{ sqlParams: [.params.id], sqlQueryName: "userProjects" }`
  |> pg: `SELECT * FROM projects WHERE user_id = $1`  # Stored as data.userProjects
```

### Phase 3: Documentation and Examples (Week 3)

#### 1. Update CLAUDE.md
- Document `sqlQueryName` field usage
- Provide examples of multiple query patterns
- Update middleware interface documentation

#### 2. Create Migration Examples
- Show before/after syntax patterns
- Provide common use case examples

## Backward Compatibility Strategy

### Graceful Fallback
- **Automatic naming**: Variables (e.g., `|> middleware: variableName`) auto-use variable name as data key
- **Explicit naming**: `resultName` field can override auto-naming
- **Legacy support**: Inline middleware calls without naming use current behavior (store result directly in `data`)
- **Priority order**: `resultName` > variable name > direct storage
- **Generic approach**: Works with any middleware (pg, http, redis, etc.)
- Existing applications continue to work unchanged
- New applications get auto-naming benefits when using variables

### Migration Path
1. **Phase 1**: Add `resultName` support (backward compatible)
2. **Phase 2**: Update examples and documentation 
3. **Phase 3**: Recommend new pattern for new applications
4. **Future**: Potentially deprecate unnamed results (with long notice)

## Expected Behavior Examples

### Single Result (Current vs Named)
```wp
# Without resultName (current behavior unchanged)
GET /user/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  # Response: { 
  #   "method": "GET", "params": {"id": "123"}, ...,
  #   "sqlParams": [...], 
  #   "data": { "rows": [...], "rowCount": 1 } 
  # }

# With optional resultName
GET /user/:id
  |> jq: `{ sqlParams: [.params.id], resultName: "profile" }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  # Response: { 
  #   "method": "GET", "params": {"id": "123"}, ...,
  #   "sqlParams": [...],
  #   "data": { "profile": { "rows": [...], "rowCount": 1 } }
  # }
```

### Multiple Results with Auto-Naming
```wp
# Using variables for automatic naming (works with any middleware)
pg getUserQuery = `SELECT * FROM users WHERE id = $1`
pg getTodosQuery = `SELECT * FROM todos WHERE user_id = $1`

GET /user/:id/dashboard
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: getUserQuery  # Auto-stored as data.getUserQuery
  |> jq: `{ sqlParams: [.params.id] }`  
  |> pg: getTodosQuery  # Auto-stored as data.getTodosQuery
  |> jq: `{
      user: .data.getUserQuery.rows[0],
      todoCount: (.data.getTodosQuery.rows | length),
      todos: .data.getTodosQuery.rows
    }`

# Mixed inline calls with explicit naming
GET /user/:id/full-dashboard
  |> jq: `{ sqlParams: [.params.id], resultName: "profile" }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `{ sqlParams: [.params.id], resultName: "tasks" }`  
  |> pg: `SELECT * FROM todos WHERE user_id = $1`
  |> jq: `{ sqlParams: [.params.id], resultName: "projects" }`
  |> pg: `SELECT * FROM projects WHERE owner_id = $1`
  |> jq: `{
      user: .data.profile.rows[0],
      tasks: .data.tasks.rows,
      projects: .data.projects.rows
    }`

# Generic middleware example (future expansion)
http getApiData = `https://api.example.com/users/{id}`

GET /user/:id/mixed
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: getUserQuery        # Auto-stored as data.getUserQuery
  |> jq: `{ url: "https://api.example.com/users/" + (.params.id | tostring) }`
  |> http: getApiData        # Auto-stored as data.getApiData (future)
```

## Success Metrics

1. **Functionality**
   - ✅ Named queries work as specified
   - ✅ Request context preserved throughout pipeline
   - ✅ Multiple queries can coexist without conflicts
   - ✅ Backward compatibility maintained

2. **Performance**  
   - ✅ No significant performance regression
   - ✅ Memory usage remains efficient
   - ✅ No memory leaks in named query scenarios

3. **Developer Experience**
   - ✅ Simple, intuitive syntax
   - ✅ Clear examples and documentation
   - ✅ Easy migration path from current patterns