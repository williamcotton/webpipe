# Result Block Implementation Plan

## Core Concept

The result block is **not a plugin** but a built-in language construct that handles conditional response formatting based on standardized error structures in the JSON object.

## Error Detection Strategy

Use a standardized error format that any plugin can return. Only `type` is required:

```json
{
  "errors": [
    {
      "type": "validationError",
      "fields": ["name", "email"],
      "rules": {"name": "required", "email": "format"}
    }
  ]
}
```

Or for SQL errors:
```json
{
  "errors": [
    {
      "type": "sqlError",
      "code": "23505",
      "constraint": "users_email_key",
      "query": "insert into users..."
    }
  ]
}
```

Or for success (no errors array):
```json
{
  "data": { "user": { "id": 1, "name": "John" } }
}
```

## Implementation Architecture

### 1. Error Detection Logic

```c
typedef struct {
    char* type;
    json_t* error_data;
} parsed_error_t;

bool has_errors(json_t* json) {
    json_t* errors = json_object_get(json, "errors");
    return errors != NULL && json_is_array(errors) && json_array_size(errors) > 0;
}

parsed_error_t* get_first_error(json_t* json, arena_t* arena) {
    json_t* errors = json_object_get(json, "errors");
    if (!errors || !json_is_array(errors)) return NULL;
    
    json_t* first_error = json_array_get(errors, 0);
    if (!first_error) return NULL;
    
    json_t* type_json = json_object_get(first_error, "type");
    if (!type_json || !json_is_string(type_json)) return NULL;
    
    parsed_error_t* error = arena_alloc(arena, sizeof(parsed_error_t));
    error->type = arena_strdup(arena, json_string_value(type_json));
    error->error_data = first_error;
    
    return error;
}
```

### 2. Pipeline Integration

```c
json_t* execute_pipeline(pipeline_t* pipeline, json_t* request, arena_t* arena) {
    json_t* current = request;
    
    for (int i = 0; i < pipeline->step_count; i++) {
        current = execute_step(pipeline->steps[i], current, arena);
        
        // Check for errors after each step
        if (pipeline->has_result_block && has_errors(current)) {
            return execute_result_block(pipeline->result_block, current, arena);
        }
    }
    
    // Execute result block for success case or if no errors
    if (pipeline->has_result_block) {
        return execute_result_block(pipeline->result_block, current, arena);
    }
    
    return current;
}
```

### 3. Result Block Execution

```c
json_t* execute_result_block(result_block_t* result_block, json_t* json, arena_t* arena) {
    // Check for errors first
    if (has_errors(json)) {
        parsed_error_t* error = get_first_error(json, arena);
        if (error) {
            // Find matching error condition
            for (int i = 0; i < result_block->condition_count; i++) {
                result_condition_t* condition = &result_block->conditions[i];
                
                if (strcmp(condition->condition_name, error->type) == 0) {
                    json_t* response = execute_pipeline(condition->response_pipeline, json, arena);
                    json_object_set_new(response, "_http_status", json_integer(condition->http_status));
                    return response;
                }
            }
        }
        
        // No matching error condition, use default error
        return execute_default_error_condition(result_block, json, arena);
    }
    
    // No errors, look for success condition
    for (int i = 0; i < result_block->condition_count; i++) {
        result_condition_t* condition = &result_block->conditions[i];
        
        if (strcmp(condition->condition_name, "ok") == 0) {
            json_t* response = execute_pipeline(condition->response_pipeline, json, arena);
            json_object_set_new(response, "_http_status", json_integer(condition->http_status));
            return response;
        }
    }
    
    // No success condition defined, return as-is
    return json;
}
```

### 4. AST Structure

```c
typedef struct {
    char* condition_name;     // "ok", "validationError", "notFound", etc.
    int http_status;          // 200, 400, 404, etc.
    pipeline_t* response_pipeline;
} result_condition_t;

typedef struct {
    result_condition_t* conditions;
    int condition_count;
    result_condition_t* default_condition;  // for unmatched error types
} result_block_t;
```

## Plugin Error Response Format

### Validation Plugin Example
```c
// In validation plugin
json_t* validate_request(json_t* request, arena_t* arena) {
    json_t* errors_array = json_array();
    
    // Check validation rules...
    if (name_invalid) {
        json_t* error = json_object();
        json_object_set_new(error, "type", json_string("validationError"));
        json_object_set_new(error, "field", json_string("name"));
        json_object_set_new(error, "rule", json_string("required"));
        json_object_set_new(error, "value", json_string(""));
        json_array_append_new(errors_array, error);
    }
    
    if (json_array_size(errors_array) > 0) {
        json_object_set_new(request, "errors", errors_array);
    }
    
    return request;
}
```

### Database Plugin Example
```c
// In postgres plugin
json_t* execute_query(json_t* request, arena_t* arena) {
    PGresult* result = PQexec(conn, query);
    
    if (PQresultStatus(result) != PGRES_TUPLES_OK) {
        json_t* errors_array = json_array();
        json_t* error = json_object();
        json_object_set_new(error, "type", json_string("sqlError"));
        json_object_set_new(error, "sqlstate", json_string(PQresultErrorField(result, PG_DIAG_SQLSTATE)));
        json_object_set_new(error, "message", json_string(PQerrorMessage(conn)));
        json_object_set_new(error, "query", json_string(query));
        json_array_append_new(errors_array, error);
        json_object_set_new(request, "errors", errors_array);
        return request;
    }
    
    if (PQntuples(result) == 0) {
        json_t* errors_array = json_array();
        json_t* error = json_object();
        json_object_set_new(error, "type", json_string("notFound"));
        json_object_set_new(error, "table", json_string("users"));
        json_object_set_new(error, "attempted_query", json_string(query));
        json_array_append_new(errors_array, error);
        json_object_set_new(request, "errors", errors_array);
        return request;
    }
    
    // Success case - add data
    json_object_set_new(request, "data", convert_pg_result_to_json(result, arena));
    return request;
}
```

### Auth Plugin Example
```c
// In auth plugin
json_t* check_auth(json_t* request, arena_t* arena) {
    char* token = get_auth_token(request);
    
    if (!token) {
        json_t* errors_array = json_array();
        json_t* error = json_object();
        json_object_set_new(error, "type", json_string("authRequired"));
        json_object_set_new(error, "header", json_string("Authorization"));
        json_object_set_new(error, "expected_format", json_string("Bearer <token>"));
        json_array_append_new(errors_array, error);
        json_object_set_new(request, "errors", errors_array);
        return request;
    }
    
    user_t* user = verify_token(token);
    if (!user) {
        json_t* errors_array = json_array();
        json_t* error = json_object();
        json_object_set_new(error, "type", json_string("authInvalid"));
        json_object_set_new(error, "token_expired", json_boolean(is_token_expired(token)));
        json_object_set_new(error, "issued_at", json_integer(get_token_issued_at(token)));
        json_array_append_new(errors_array, error);
        json_object_set_new(request, "errors", errors_array);
        return request;
    }
    
    // Success - add user to request
    json_object_set_new(request, "user", user_to_json(user, arena));
    return request;
}
```

## Example Usage

```wp
POST /api/users
  |> validate: `
    name: string(2..50)
    email: email
  `
  |> lua: `return { sqlParams = { body.name, body.email } }`
  |> pg: `insert into users (name, email) values ($1, $2) returning *`
  |> result
    ok(201): 
      |> jq: `{success: true, user: .data.rows[0]}`
    validationError(400): 
      |> jq: `{
        error: "Validation failed", 
        field: .errors[0].field,
        rule: .errors[0].rule,
        value: .errors[0].value
      }`
    sqlError(500):
      |> jq: `{
        error: "Database error",
        sqlstate: .errors[0].sqlstate,
        message: .errors[0].message,
        query: .errors[0].query
      }`
    notFound(404):
      |> jq: `{
        error: "Resource not found",
        table: .errors[0].table,
        query: .errors[0].attempted_query
      }`
    authRequired(401):
      |> jq: `{
        error: "Authentication required",
        header: .errors[0].header,
        format: .errors[0].expected_format
      }`
    authInvalid(401): 
      |> jq: `{
        error: "Invalid authentication",
        expired: .errors[0].token_expired,
        issued_at: .errors[0].issued_at
      }`
    default(500):
      |> jq: `{error: "Internal server error"}`
```

## Benefits

1. **Standardized Error Format**: All plugins use the same error structure
2. **Flexible Error Types**: Any plugin can define custom error types
3. **No Hardcoding**: Runtime doesn't need to know about specific error types
4. **Multiple Errors**: Can handle multiple errors in one response
5. **Rich Error Data**: Each error can contain additional context
6. **Early Termination**: Pipeline stops on first error
7. **Type Safety**: Error types are matched exactly against result conditions

## Error Precedence

1. **First Error Wins**: If multiple errors exist, use the first one in the array
2. **Unmatched Types**: If error type doesn't match any condition, use default
3. **No Default**: If no default and no match, return 500 with generic error message

This approach gives maximum flexibility while maintaining a clean, standardized interface between plugins and the result block system.