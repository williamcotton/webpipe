# Content Type Support in BDD Testing Framework

## Problem Statement

The current BDD testing framework only supports JSON output validation. When testing routes that return HTML content (via mustache middleware), the test fails because the framework attempts to parse HTML as JSON:

```
hello, world
Failed to parse expected JSON: '[' or '{' expected near '<'
  ✗ calls the route
```

The failing test case:
```wp
GET /hello/:world
  |> jq: `{ world: .params.world }`
  |> mustache: `<p>hello, {{world}}</p>`

describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and output equals `<p>hello, world</p>`
```

## Current Implementation Analysis

### Key Files and Functions

- **src/testing.c**: Main testing framework implementation
- **validate_assertions()** (lines 201-240): The problematic function that only handles JSON
- **execute_route_test()** (lines 410-445): Executes route tests and captures response
- **execute_route_pipeline()** (lines 382-408): Pipeline execution with content-type handling

### Current Flow

1. Route pipeline executes and produces final response
2. `execute_route_pipeline()` calls `execute_pipeline_with_result()` 
3. Content type is captured in `content_type` variable but not used in testing
4. `validate_assertions()` assumes all output is JSON and calls `json_loads()`
5. HTML content fails JSON parsing

## Solution Design

### 1. Content-Type Detection Strategy

The testing framework needs to detect content type and handle different response formats:

**Detection Methods:**
- **Primary**: Use content-type returned by `execute_pipeline_with_result()`
- **Fallback**: Attempt JSON parsing; if it fails, treat as text content
- **Heuristic**: Check if expected output starts with `<` (HTML) or `{`/`[` (JSON)

### 2. Enhanced Assertion Types

Extend the current `TEST_ASSERT_OUTPUT_EQUALS` to support multiple content types:

**New Assertion Behavior:**
- **JSON Content**: Use existing `json_equal()` comparison
- **HTML/Text Content**: Use string comparison with whitespace normalization
- **Mixed Support**: Auto-detect based on content-type and content structure

### 3. Implementation Changes

#### A. Modify `validate_assertions()` Function

**Current signature:**
```c
bool validate_assertions(ASTNode **assertions, int assertion_count, 
                        json_t *result, int status_code)
```

**New signature:**
```c
bool validate_assertions(ASTNode **assertions, int assertion_count, 
                        json_t *result, int status_code, 
                        const char *content_type)
```

#### B. Update Pipeline Execution Functions

**Modify `execute_route_test()` to pass content-type:**
- Capture `content_type` from `execute_route_pipeline()`
- Pass to `validate_assertions()`

**Modify `execute_route_pipeline()` to return content-type:**
- Update return mechanism to include content-type information
- Ensure content-type is properly captured from pipeline execution

#### C. Enhanced Output Comparison Logic

**For JSON content (application/json):**
```c
// Existing logic
json_t *expected = json_loads(expected_string, 0, &error);
bool matches = json_equal(result, expected);
```

**For HTML/Text content (text/html, text/plain, etc.):**
```c
// New logic for text content
char *result_text = extract_text_from_json(result);
char *normalized_expected = normalize_whitespace(expected_string);
char *normalized_result = normalize_whitespace(result_text);
bool matches = strcmp(normalized_expected, normalized_result) == 0;
```

### 4. Response Extraction Logic

The testing framework needs to extract the actual response content based on the pipeline result:

**For JSON responses:**
- Use the JSON object directly

**For HTML/Text responses:**
- Extract string value from JSON response (middleware returns text as JSON string)
- Handle cases where response is wrapped in JSON structure

### 5. Whitespace and Formatting Handling

**HTML Content Normalization:**
- Trim leading/trailing whitespace
- Normalize internal whitespace (multiple spaces → single space)
- Preserve HTML structure while allowing for formatting differences

**Example:**
```c
char* normalize_html_whitespace(const char* html) {
    // Remove leading/trailing whitespace
    // Collapse multiple spaces between tags
    // Preserve single spaces within content
}
```

## Implementation Plan

### Phase 1: Core Infrastructure
1. **Modify function signatures** to pass content-type through the testing pipeline
2. **Add content-type detection** in `execute_route_test()`
3. **Create helper functions** for text extraction and normalization

### Phase 2: Enhanced Assertion Logic
1. **Update `validate_assertions()`** to handle multiple content types
2. **Implement text comparison** logic with normalization
3. **Add fallback detection** for when content-type is not available

### Phase 3: Testing and Refinement
1. **Test with existing JSON tests** to ensure backward compatibility
2. **Test with HTML content** like the mustache example
3. **Add mixed content-type test cases**

### Phase 4: Documentation and Error Messages
1. **Improve error messages** to show content-type information
2. **Update test output** to indicate content-type being tested
3. **Add debugging information** for content-type detection

## Backward Compatibility

**Existing JSON tests** will continue to work because:
- JSON content-type detection will use existing logic
- Function signatures maintain compatibility via overloading or optional parameters
- Default behavior assumes JSON when content-type is unknown

## Enhanced Test Output

**Before:**
```
hello, world
Failed to parse expected JSON: '[' or '{' expected near '<'
  ✗ calls the route
```

**After:**
```
hello, world
Content-Type: text/html
Expected: <p>hello, world</p>
Got:      <p>hello, world</p>
  ✓ calls the route
```

## Additional Benefits

1. **Support for CSS/JS testing** (text/css, text/javascript)
2. **API documentation testing** (text/plain responses)
3. **Mixed content pipelines** (JSON → HTML transformation testing)
4. **Better error diagnostics** with content-type information

## Test Cases to Support

**HTML Content:**
```wp
describe "mustache rendering"
  it "renders HTML template"
    when calling GET /hello/world
    then status is 200
    and contentType is "text/html"
    and output equals `<p>hello, world</p>`
```

**Plain Text:**
```wp
describe "plain text response"
  it "returns text content"
    when calling GET /api/status
    then status is 200
    and contentType is "text/plain"
    and output equals `Server is running`
```

**JSON (existing behavior):**
```wp
describe "JSON API"
  it "returns JSON data"
    when calling GET /api/users
    then status is 200
    and contentType is "application/json"
    and output equals `{"users": []}`
```

This implementation will allow the failing test case to pass while maintaining full backward compatibility with existing JSON tests.