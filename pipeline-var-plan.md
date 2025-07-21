# Pipeline Variable Implementation Plan

## Current State Analysis

Based on analysis of the codebase (server.c, parser.c, lexer.c, test.wp), the webpipe runtime currently supports:

1. **Variable assignments**: `pg articlesQuery = 'SELECT * FROM articles'`
2. **Variable usage in pipelines**: `|> pg: articlesQuery`
3. **Runtime variable storage**: Variables are stored in `runtime->variables` as JSON object
4. **Pipeline execution**: Variables are resolved during pipeline execution via `step->is_variable` flag

## Proposed Pipeline Variable Syntax

The new syntax should support:

```wp
pipeline getPage = 
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: `SELECT * FROM teams WHERE id = $1`
  |> jq: `{ team: .data.rows[0] }`

GET /page/:id
  |> pipeline: getPage
```

## Implementation Plan

### 1. Lexer Changes (`src/lexer.c`)

**Status**: Minimal changes needed
- The lexer already supports all required tokens: `IDENTIFIER`, `EQUALS`, `STRING`, `PIPE`, etc.
- No new token types are required for pipeline variables

### 2. Parser Changes (`src/parser.c`)

**Changes needed**:

#### 2.1 New AST Node Type
Add `AST_PIPELINE_DEFINITION` to support pipeline variable assignments:

```c
typedef struct {
    char *name;
    PipelineStep *pipeline;
} PipelineDefinition;
```

#### 2.2 Parser Function Updates
- Modify `parser_parse_variable_assignment()` to detect pipeline variables
- Add `parser_parse_pipeline_definition()` function
- Update `parser_parse_statement()` to handle pipeline definitions
- The existing pipeline parsing logic in `parser_parse_pipeline()` can be reused

#### 2.3 Pipeline Middleware Handling
Add logic to handle `pipeline` as a special middleware type:
- When `step->middleware == "pipeline"`, resolve the pipeline variable name
- Replace the pipeline step with the actual pipeline steps from the variable

### 3. Runtime Changes (`src/server.c`)

**Changes needed**:

#### 3.1 Pipeline Variable Storage
- Extend `runtime->variables` JSON object to store pipeline definitions
- Store pipeline variables differently from regular string variables
- Consider using a separate `runtime->pipelines` hashtable for better performance

#### 3.2 Pipeline Execution Updates
Modify `execute_pipeline_with_result()` to handle pipeline variables:

```c
// When encountering pipeline middleware
if (strcmp(step->middleware, "pipeline") == 0) {
    // Look up pipeline definition
    PipelineStep *pipeline_steps = find_pipeline_variable(step->value);
    if (pipeline_steps) {
        // Execute the pipeline steps inline
        // Continue with remaining steps after pipeline execution
    }
}
```

#### 3.3 Pipeline Resolution
Add function to resolve pipeline variables recursively:
- Handle nested pipeline references
- Detect circular dependencies
- Cache resolved pipelines for performance

### 4. AST Processing (`src/parser.c`)

**Changes needed**:

#### 4.1 Variable Processing Updates
Modify the variable processing loop in `wp_runtime_init()`:

```c
// Process pipeline definitions
for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
    ASTNode *stmt = runtime->program->data.program.statements[i];
    if (stmt->type == AST_PIPELINE_DEFINITION) {
        // Store pipeline in runtime->pipelines or runtime->variables
        store_pipeline_variable(stmt->data.pipeline_def.name, 
                              stmt->data.pipeline_def.pipeline);
    }
}
```

### 5. Data Structures (`src/wp.h`)

**New structures needed**:

```c
// Add to ASTNodeType enum
typedef enum {
    // ... existing types
    AST_PIPELINE_DEFINITION
} ASTNodeType;

// Add to ASTNode union
typedef struct ASTNode {
    ASTNodeType type;
    union {
        // ... existing members
        struct {
            char *name;
            PipelineStep *pipeline;
        } pipeline_def;
    } data;
} ASTNode;
```

### 6. Memory Management

**Considerations**:
- Pipeline variables should use the same arena allocation as other variables
- Pipeline steps within variables need proper cleanup in `free_ast()`
- Consider pipeline variable lifetime (program-scoped like other variables)

### 7. Error Handling

**New error cases**:
- Undefined pipeline variable reference
- Circular pipeline dependencies
- Invalid pipeline syntax in variable assignment

### 8. Testing Strategy

**Test cases needed**:
- Basic pipeline variable definition and usage
- Nested pipeline variables
- Pipeline variable with different middleware combinations
- Error cases (undefined variables, circular references)
- Integration with existing variable system

## Implementation Priority

1. **Phase 1**: Parser changes to recognize pipeline syntax
2. **Phase 2**: AST node types and basic pipeline storage
3. **Phase 3**: Runtime pipeline resolution and execution
4. **Phase 4**: Error handling and edge cases
5. **Phase 5**: Performance optimization and testing

## Compatibility

This implementation maintains full backward compatibility:
- Existing variable assignments continue to work unchanged
- No changes to existing pipeline syntax
- New `pipeline` middleware type is additive

## Performance Considerations

- Pipeline variable resolution happens at runtime during execution
- Consider caching resolved pipeline steps to avoid repeated parsing
- Pipeline variables are resolved once per request (per current variable system)
- Memory usage increase is minimal (pipeline definitions stored in parse arena)

## Example Usage After Implementation

```wp
# Define reusable pipeline
pipeline getTeamById = 
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: `SELECT * FROM teams WHERE id = $1`
  |> jq: `{ team: .data.rows[0] }`

# Define another pipeline that uses the first one
pipeline getTeamWithMembers = 
  |> pipeline: getTeamById
  |> jq: `{ sqlParams: [.team.id | tostring] }`
  |> pg: `SELECT * FROM team_members WHERE team_id = $1`
  |> jq: `{ team: .team, members: .data.rows }`

# Use pipeline in routes
GET /page/:id
  |> pipeline: getTeamById

GET /team/:id/members
  |> pipeline: getTeamWithMembers
```

This implementation provides a clean, reusable way to define common pipeline patterns while maintaining the existing webpipe architecture and design principles.