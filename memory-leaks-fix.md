# Memory Leaks Fix Plan

## Current Memory Leak Analysis

Based on the leaks report, we have **75 leaks totaling 3,392 bytes**, primarily from:

1. **Parser Result Steps** (70 leaks, 2.94K): All occurring in `parser_parse_result_step` and related functions
2. **JSON Objects** (5 leaks, 384 bytes): Root cycle in JSON object creation during runtime initialization

### Leak Locations:
- `parser_parse_result_step` - Multiple ResultCondition allocations
- `parser_parse_pipeline` - PipelineStep allocations  
- `strdup_safe` - String duplications for plugin names, condition names, values
- `json_object` - JSON object cycles in runtime variables

## Root Cause Analysis

### 1. Parser Memory Management Issues
- **No centralized cleanup**: Parser creates AST nodes but never properly frees them
- **Complex ownership**: AST nodes have nested ownership (conditions -> pipelines -> steps)
- **String allocations**: Multiple `strdup_safe` calls create orphaned strings
- **Missing free functions**: No comprehensive AST cleanup during shutdown

### 2. JSON Memory Cycles
- **Jansson arena integration**: Custom allocators create reference cycles
- **Runtime variables**: JSON objects stored in runtime never freed
- **Arena cleanup**: Arena memory not properly released

## Solution: Parser Memory Arena

### Core Strategy
Replace the current ad-hoc memory allocation with a **dedicated parser arena** that can be freed all at once, eliminating the need for complex tree-walking cleanup functions.

### Implementation Plan

#### Phase 1: Parser Arena Infrastructure

```c
// New parser arena structure
typedef struct {
    MemoryArena *parse_arena;    // For AST nodes, strings, conditions
    MemoryArena *runtime_arena;  // For long-lived runtime data
} ParseContext;
```

**Benefits:**
- Single allocation point for all parser data
- Atomic cleanup - free entire arena at once
- No need for complex `free_ast()` functions
- Eliminates manual memory tracking

#### Phase 2: Arena-Based String Allocation

**Replace `strdup_safe()` with arena-based allocation:**

```c
// Current problematic pattern:
char *value = strdup_safe(token->value);  // malloc() - never freed

// New arena-based pattern:
char *value = arena_strdup(parse_context->parse_arena, token->value);  // arena alloc
```

**Implementation steps:**
1. Add `arena_strdup()` function to arena.c
2. Replace all `strdup_safe()` calls in parser.c with `arena_strdup()`
3. Pass parse context through all parser functions

#### Phase 3: Arena-Based AST Node Allocation

**Convert all AST allocations to use arena:**

```c
// Current problematic pattern:
ASTNode *node = malloc(sizeof(ASTNode));           // malloc() - never freed
ResultCondition *condition = malloc(sizeof(ResultCondition));  // malloc() - never freed

// New arena-based pattern:
ASTNode *node = arena_alloc(parse_context->parse_arena, sizeof(ASTNode));
ResultCondition *condition = arena_alloc(parse_context->parse_arena, sizeof(ResultCondition));
```

#### Phase 4: Lifecycle Management

**Parse Context Creation:**
```c
ParseContext *parse_context_create(void) {
    ParseContext *ctx = malloc(sizeof(ParseContext));
    ctx->parse_arena = arena_create(1024 * 1024);  // 1MB for parsing
    ctx->runtime_arena = arena_create(256 * 1024); // 256KB for runtime data
    return ctx;
}

void parse_context_destroy(ParseContext *ctx) {
    arena_free(ctx->parse_arena);   // Frees ALL parser allocations at once
    arena_free(ctx->runtime_arena); // Frees runtime data
    free(ctx);
}
```

**Integration with wp_runtime_init:**
```c
int wp_runtime_init(const char *wp_file) {
    ParseContext *parse_ctx = parse_context_create();
    
    // Parse with arena
    Parser *parser = parser_new_with_context(tokens, token_count, parse_ctx);
    runtime->program = parser_parse(parser);
    
    // Copy essential data to runtime arena if needed
    // Then cleanup parse arena
    arena_free(parse_ctx->parse_arena);  // Free ALL parser memory
    parse_ctx->parse_arena = NULL;
    
    // Keep runtime arena for variables, etc.
    runtime->parse_context = parse_ctx;
}
```

## Specific Leak Fixes

### 1. Result Step Leaks (parser.c:174)

**Current problematic code:**
```c
ResultCondition *condition = malloc(sizeof(ResultCondition));
condition->condition_name = strdup_safe(condition_name->value);
// ... never freed
```

**Fixed with arena:**
```c
ResultCondition *condition = arena_alloc(ctx->parse_arena, sizeof(ResultCondition));
condition->condition_name = arena_strdup(ctx->parse_arena, condition_name->value);
// ... automatically freed when arena is destroyed
```

### 2. Pipeline Step Leaks

**Current problematic code:**
```c
PipelineStep *step = malloc(sizeof(PipelineStep));
step->plugin = strdup_safe(plugin->value);
step->value = strdup_safe(value);
// ... never freed
```

**Fixed with arena:**
```c
PipelineStep *step = arena_alloc(ctx->parse_arena, sizeof(PipelineStep));
step->plugin = arena_strdup(ctx->parse_arena, plugin->value);
step->value = arena_strdup(ctx->parse_arena, value);
// ... automatically freed when arena is destroyed
```

### 3. JSON Runtime Variables Cycle

**Current problematic code:**
```c
runtime->variables = json_object();  // Never freed, creates cycles
```

**Fixed approach:**
```c
// Use separate arena for runtime JSON data
set_current_arena(parse_ctx->runtime_arena);
runtime->variables = json_object();
// Cleanup during wp_runtime_cleanup()
```

## Implementation Steps

### Step 1: Add Arena String Functions
```c
// In arena.c
char *arena_strdup(MemoryArena *arena, const char *str);
char *arena_strndup(MemoryArena *arena, const char *str, size_t n);
```

### Step 2: Update Parser Interface
```c
// In parser.h/parser.c
typedef struct ParseContext ParseContext;
Parser *parser_new_with_context(Token *tokens, int token_count, ParseContext *ctx);
```

### Step 3: Refactor Parser Functions
- Update all `malloc()` calls to use `arena_alloc()`
- Update all `strdup_safe()` calls to use `arena_strdup()`
- Pass `ParseContext` through parser call chain
- Remove all manual `free()` calls in parser

### Step 4: Update Runtime Integration
- Create parse context in `wp_runtime_init()`
- Manage arena lifecycle properly
- Update `wp_runtime_cleanup()` to free arenas

### Step 5: Remove Manual Cleanup Functions
- Delete `free_ast()`, `free_pipeline()`, `free_result_conditions()`
- Delete `free_tokens()` (if using arena for tokens too)
- Simplify cleanup logic

## Benefits of This Approach

### 1. **Leak Elimination**
- **Impossible to leak**: Arena freed atomically
- **No manual tracking**: Arena manages all allocations
- **Simple lifecycle**: Create -> Use -> Destroy

### 2. **Performance Improvements**
- **Faster allocation**: Arena allocation is faster than malloc
- **Better locality**: Related data allocated together
- **Reduced fragmentation**: Large contiguous blocks

### 3. **Code Simplification**
- **No complex cleanup**: Eliminate tree-walking free functions
- **Clear ownership**: Arena owns all parser data
- **Easier debugging**: Single allocation/deallocation point

### 4. **Maintainability**
- **Less error-prone**: Can't forget to free individual items
- **Clearer code**: Allocation strategy is explicit
- **Future-proof**: Easy to add new AST node types

## Testing Strategy

### 1. **Memory Leak Verification**
```bash
# Before fix
make leaks
# Should show 75 leaks

# After fix
make leaks
# Should show 0 leaks
```

### 2. **Functionality Testing**
- Verify all parser functionality still works
- Test complex result blocks
- Test multiple route definitions
- Test variable assignments

### 3. **Performance Testing**
- Measure parsing time before/after
- Monitor memory usage patterns
- Verify arena utilization

## Risk Mitigation

### 1. **Backward Compatibility**
- Keep existing parser interface as wrapper
- Gradual migration approach possible
- Maintain same AST structure

### 2. **Arena Size Management**
- Start with conservative 1MB arena
- Add arena growth if needed
- Monitor arena utilization

### 3. **Error Handling**
- Graceful arena allocation failure
- Proper cleanup on parse errors
- Memory pressure handling

## Expected Results

After implementation:
- ✅ **0 memory leaks** in parser
- ✅ **Simplified code** - no manual cleanup
- ✅ **Better performance** - faster allocation
- ✅ **Maintainable** - clear ownership model
- ✅ **Robust** - impossible to forget cleanup

This approach transforms memory management from error-prone manual tracking to automatic, leak-proof arena management.