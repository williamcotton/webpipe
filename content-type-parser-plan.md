# Content-Type Parser and AST Enhancement Plan

## Overview

After analyzing the lexer and parser implementation, significant changes are needed to support content-type assertions in the BDD testing framework. The current system only supports `output equals` and `status is` assertions, but we need to add `contentType is` assertions and enhanced output handling for non-JSON content.

## Current Implementation Analysis

### Lexer.c (lines 97-161)
- **Token Recognition**: Handles testing keywords like `describe`, `it`, `when`, `then`, `output`, `equals`, `status`, `is`, `and`
- **String Handling**: Uses backticks and quotes for string literals
- **Identifier Parsing**: Recognizes keywords and converts them to specific token types
- **Route Support**: Parses HTTP routes with parameters

### Parser.c Testing Functions (lines 1171-1492)
- **Current AST Nodes**: `AST_TEST_ASSERTION` with two types:
  - `TEST_ASSERT_OUTPUT_EQUALS`
  - `TEST_ASSERT_STATUS_IS`
- **Assertion Parsing**: `parser_parse_test_assertion()` handles `then`/`and` followed by assertion types
- **Structure**: Fixed assertion types with simple string/number data

## Required Changes

### 1. Lexer Enhancements

#### A. New Token Types (wp.h)
```c
// Add to TokenType enum
typedef enum {
    // ... existing tokens ...
    TOKEN_CONTENT_TYPE,    // "contentType" keyword
    TOKEN_CONTENT,         // "content" keyword (alternative to "output")
    TOKEN_MATCHES,         // "matches" keyword (for regex/pattern matching)
    TOKEN_CONTAINS,        // "contains" keyword
    TOKEN_STARTS_WITH,     // "starts" keyword
    TOKEN_ENDS_WITH,       // "ends" keyword
    // ... existing tokens ...
} TokenType;
```

#### B. Lexer Keyword Recognition (lexer.c:97-161)
```c
// Add to lexer_read_identifier() function
} else if (strcmp(value, "contentType") == 0) {
    type = TOKEN_CONTENT_TYPE;
} else if (strcmp(value, "content") == 0) {
    type = TOKEN_CONTENT;
} else if (strcmp(value, "matches") == 0) {
    type = TOKEN_MATCHES;
} else if (strcmp(value, "contains") == 0) {
    type = TOKEN_CONTAINS;
} else if (strcmp(value, "starts") == 0) {
    type = TOKEN_STARTS_WITH;
} else if (strcmp(value, "ends") == 0) {
    type = TOKEN_ENDS_WITH;
```

### 2. AST Node Enhancements

#### A. New Test Assertion Types (wp.h)
```c
// Extend TestAssertionType enum
typedef enum {
    TEST_ASSERT_OUTPUT_EQUALS,
    TEST_ASSERT_STATUS_IS,
    TEST_ASSERT_CONTENT_TYPE_IS,     // New: contentType assertion
    TEST_ASSERT_CONTENT_EQUALS,      // New: alternative to output equals
    TEST_ASSERT_CONTENT_CONTAINS,    // New: partial content matching
    TEST_ASSERT_CONTENT_MATCHES,     // New: regex/pattern matching
    TEST_ASSERT_CONTENT_STARTS_WITH, // New: prefix matching
    TEST_ASSERT_CONTENT_ENDS_WITH,   // New: suffix matching
} TestAssertionType;
```

#### B. AST Node Data Structures (wp.h)
```c
// Extend TestAssertionData union
typedef union {
    struct {
        char *expected_json;
    } output_equals;
    
    struct {
        int expected_status;
    } status_is;
    
    // New assertion data structures
    struct {
        char *expected_content_type;
    } content_type_is;
    
    struct {
        char *expected_content;
        ContentComparisonMode mode; // EXACT, NORMALIZED_WHITESPACE, IGNORE_WHITESPACE
    } content_equals;
    
    struct {
        char *expected_substring;
        bool case_sensitive;
    } content_contains;
    
    struct {
        char *pattern;
        bool case_sensitive;
    } content_matches;
    
    struct {
        char *expected_prefix;
        bool case_sensitive;
    } content_starts_with;
    
    struct {
        char *expected_suffix;
        bool case_sensitive;
    } content_ends_with;
} TestAssertionData;

// New enum for content comparison modes
typedef enum {
    CONTENT_EXACT,              // Exact string match
    CONTENT_NORMALIZED_WS,      // Normalize whitespace
    CONTENT_IGNORE_WS,         // Ignore all whitespace
    CONTENT_HTML_NORMALIZED    // HTML-specific normalization
} ContentComparisonMode;
```

### 3. Parser Function Enhancements

#### A. Enhanced Assertion Parsing (parser.c:1457-1492)

Replace `parser_parse_test_assertion()` with extended logic:

```c
ASTNode *parser_parse_test_assertion(Parser *parser) {
    if (!parser_match(parser, TOKEN_THEN) && !parser_match(parser, TOKEN_AND)) {
        return NULL;
    }

    ASTNode *node = malloc(sizeof(ASTNode));
    node->type = AST_TEST_ASSERTION;

    // Parse different assertion types
    if (parser_match(parser, TOKEN_STATUS) && parser_match(parser, TOKEN_IS)) {
        // Existing status assertion
        node->data.test_assertion.type = TEST_ASSERT_STATUS_IS;
        if (!parser_check(parser, TOKEN_NUMBER)) {
            fprintf(stderr, "Expected status code number after 'status is'\n");
            free(node);
            return NULL;
        }
        node->data.test_assertion.data.status_is.expected_status = 
            atoi(parser_advance(parser)->value);
            
    } else if (parser_match(parser, TOKEN_CONTENT_TYPE) && parser_match(parser, TOKEN_IS)) {
        // New contentType assertion
        node->data.test_assertion.type = TEST_ASSERT_CONTENT_TYPE_IS;
        if (!parser_check(parser, TOKEN_STRING)) {
            fprintf(stderr, "Expected content-type string after 'contentType is'\n");
            free(node);
            return NULL;
        }
        node->data.test_assertion.data.content_type_is.expected_content_type = 
            strdup(parser_advance(parser)->value);
            
    } else if ((parser_match(parser, TOKEN_OUTPUT) || parser_match(parser, TOKEN_CONTENT)) && 
               parser_match(parser, TOKEN_EQUALS_ASSERTION)) {
        // Enhanced output/content equals - auto-detect JSON vs HTML/text
        node->data.test_assertion.type = TEST_ASSERT_CONTENT_EQUALS;
        if (!parser_check(parser, TOKEN_STRING)) {
            fprintf(stderr, "Expected content string after 'output/content equals'\n");
            free(node);
            return NULL;
        }
        node->data.test_assertion.data.content_equals.expected_content = 
            strdup(parser_advance(parser)->value);
        node->data.test_assertion.data.content_equals.mode = CONTENT_NORMALIZED_WS;
        
    } else if ((parser_match(parser, TOKEN_OUTPUT) || parser_match(parser, TOKEN_CONTENT)) && 
               parser_match(parser, TOKEN_CONTAINS)) {
        // New content contains assertion
        node->data.test_assertion.type = TEST_ASSERT_CONTENT_CONTAINS;
        if (!parser_check(parser, TOKEN_STRING)) {
            fprintf(stderr, "Expected content string after 'output/content contains'\n");
            free(node);
            return NULL;
        }
        node->data.test_assertion.data.content_contains.expected_substring = 
            strdup(parser_advance(parser)->value);
        node->data.test_assertion.data.content_contains.case_sensitive = true;
        
    } else {
        fprintf(stderr, "Unknown assertion type\n");
        free(node);
        return NULL;
    }

    return node;
}
```

#### B. Backward Compatibility Handling
Maintain support for existing `output equals` syntax while adding new `content equals`:

```c
// Both syntaxes should work:
// then output equals `{"hello": "world"}`     // Legacy, auto-detect JSON
// then content equals `<p>hello, world</p>`  // New, auto-detect HTML/text
```

### 4. New Test Syntax Support

#### A. Enhanced BDD Test Syntax
```wp
describe "content-type testing"
  it "returns HTML content"
    when calling GET /hello/world
    then status is 200
    and contentType is "text/html"
    and content equals `<p>hello, world</p>`

  it "contains expected text"
    when calling GET /hello/world
    then content contains `hello, world`
    and content starts with `<p>`
    and content ends with `</p>`

  it "matches HTML pattern"
    when calling GET /hello/world
    then content matches `<p>hello, \w+</p>`
```

#### B. Backward Compatible Legacy Syntax
```wp
describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and output equals `<p>hello, world</p>`  # Auto-detects HTML content
```

### 5. Implementation Phases

#### Phase 1: Basic Content-Type Support
1. **Add core tokens**: `TOKEN_CONTENT_TYPE`, `TOKEN_CONTENT`
2. **Add basic AST nodes**: `TEST_ASSERT_CONTENT_TYPE_IS`, `TEST_ASSERT_CONTENT_EQUALS`
3. **Update parser**: Handle `contentType is` and `content equals` assertions
4. **Testing framework**: Pass content-type information to assertion validation

#### Phase 2: Enhanced Content Detection
1. **Auto-detection logic**: Determine content type from assertion content
2. **Backward compatibility**: Route `output equals` to appropriate handler based on content
3. **Content extraction**: Handle JSON vs HTML/text content extraction
4. **Testing framework**: Implement content comparison logic

#### Phase 3: Advanced Features  
1. **Add matching tokens**: `TOKEN_CONTAINS`, `TOKEN_MATCHES`, `TOKEN_STARTS_WITH`, `TOKEN_ENDS_WITH`
2. **Add matching AST nodes**: Content contains, matches, starts/ends with
3. **Update parser**: Handle new assertion types
4. **Testing framework**: Implement content matching logic

### 6. Testing Framework Integration

#### A. Enhanced validate_assertions() Function (testing.c:201-240)
```c
bool validate_assertions(ASTNode **assertions, int assertion_count, 
                        json_t *result, int status_code, 
                        const char *content_type) {
    for (int i = 0; i < assertion_count; i++) {
        ASTNode *assertion = assertions[i];
        if (assertion->type == AST_TEST_ASSERTION) {
            switch (assertion->data.test_assertion.type) {
                case TEST_ASSERT_STATUS_IS:
                    // Existing status validation
                    break;
                    
                case TEST_ASSERT_CONTENT_TYPE_IS: {
                    const char *expected = assertion->data.test_assertion.data.content_type_is.expected_content_type;
                    if (!content_type || strcmp(content_type, expected) != 0) {
                        printf("    %s❌ Content-Type assertion failed:%s\n", ANSI_RED, ANSI_RESET);
                        printf("    %sExpected:%s %s\n", ANSI_YELLOW, ANSI_RESET, expected);
                        printf("    %sGot:     %s %s\n", ANSI_YELLOW, ANSI_RESET, content_type ? content_type : "null");
                        return false;
                    }
                    break;
                }
                
                case TEST_ASSERT_CONTENT_EQUALS:
                case TEST_ASSERT_OUTPUT_EQUALS: {
                    // Unified content/output handling with auto-detection
                    char *actual_content = extract_content_from_result(result, content_type);
                    char *expected = (assertion->data.test_assertion.type == TEST_ASSERT_CONTENT_EQUALS) ?
                        assertion->data.test_assertion.data.content_equals.expected_content :
                        assertion->data.test_assertion.data.output_equals.expected_json;
                    
                    ContentComparisonMode mode = detect_comparison_mode(expected, content_type);
                    
                    if (!compare_content(actual_content, expected, mode)) {
                        printf("    %s❌ Content assertion failed:%s\n", ANSI_RED, ANSI_RESET);
                        printf("    %sExpected:%s %s\n", ANSI_YELLOW, ANSI_RESET, expected);
                        printf("    %sGot:     %s %s\n", ANSI_YELLOW, ANSI_RESET, actual_content);
                        return false;
                    }
                    break;
                }
            }
        }
    }
    return true;
}
```

#### B. Content Extraction and Comparison
```c
// Extract content based on content-type
char *extract_content_from_result(json_t *result, const char *content_type) {
    if (!result) return NULL;
    
    if (content_type && strstr(content_type, "application/json")) {
        // For JSON, serialize the object
        return json_dumps(result, JSON_INDENT(2));
    } else if (content_type && (strstr(content_type, "text/html") || 
                               strstr(content_type, "text/plain"))) {
        // For HTML/text, extract string value
        if (json_is_string(result)) {
            return strdup(json_string_value(result));
        }
    }
    
    // Fallback: serialize as JSON
    return json_dumps(result, JSON_COMPACT);
}

// Auto-detect comparison mode based on content and content-type
ContentComparisonMode detect_comparison_mode(const char *expected, const char *content_type) {
    // If content-type indicates JSON, use exact matching
    if (content_type && strstr(content_type, "application/json")) {
        return CONTENT_EXACT;
    }
    
    // If expected content looks like JSON, use exact matching
    if (expected[0] == '{' || expected[0] == '[') {
        return CONTENT_EXACT;
    }
    
    // If expected content looks like HTML, use HTML normalization
    if (expected[0] == '<') {
        return CONTENT_HTML_NORMALIZED;
    }
    
    // Default to normalized whitespace for text content
    return CONTENT_NORMALIZED_WS;
}

// Compare content with different modes
bool compare_content(const char *actual, const char *expected, ContentComparisonMode mode) {
    switch (mode) {
        case CONTENT_EXACT:
            return strcmp(actual, expected) == 0;
            
        case CONTENT_NORMALIZED_WS: {
            char *norm_actual = normalize_whitespace(actual);
            char *norm_expected = normalize_whitespace(expected);
            bool result = strcmp(norm_actual, norm_expected) == 0;
            free(norm_actual);
            free(norm_expected);
            return result;
        }
        
        case CONTENT_HTML_NORMALIZED: {
            char *norm_actual = normalize_html_content(actual);
            char *norm_expected = normalize_html_content(expected);
            bool result = strcmp(norm_actual, norm_expected) == 0;
            free(norm_actual);
            free(norm_expected);
            return result;
        }
        
        default:
            return false;
    }
}

// Normalize whitespace for comparison
char *normalize_whitespace(const char *str) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    char *normalized = malloc(len + 1);
    char *dst = normalized;
    bool in_whitespace = false;
    
    // Trim leading whitespace
    while (*str && isspace(*str)) str++;
    
    while (*str) {
        if (isspace(*str)) {
            if (!in_whitespace) {
                *dst++ = ' ';
                in_whitespace = true;
            }
        } else {
            *dst++ = *str;
            in_whitespace = false;
        }
        str++;
    }
    
    // Trim trailing whitespace
    while (dst > normalized && isspace(*(dst-1))) dst--;
    
    *dst = '\0';
    return normalized;
}
```

### 7. Backward Compatibility

#### A. Legacy Support
- **Existing tests**: `output equals` continues to work for both JSON and HTML/text
- **Auto-detection**: Content type automatically determined from assertion content
- **Parser compatibility**: Old syntax routes to appropriate handlers
- **Gradual migration**: Users can adopt new syntax incrementally

#### B. Migration Path
```wp
# Current failing test (fixed automatically):
describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and output equals `<p>hello, world</p>`  # Now works with HTML content

# Enhanced version with explicit content-type checking:
describe "hello, world"
  it "calls the route"
    when calling GET /hello/world
    then status is 200
    and contentType is "text/html"
    and content equals `<p>hello, world</p>`
```

### 8. Implementation Priority

#### Immediate (Phase 1) - Fix Current Issue
1. **Enhanced content detection** in `validate_assertions()`
2. **Content extraction logic** for HTML/text vs JSON
3. **Whitespace normalization** for HTML content comparison
4. **No parser changes needed** - work with existing `output equals`

#### Short Term (Phase 2) - Add contentType Assertions
1. **Add `TOKEN_CONTENT_TYPE`** to lexer
2. **Add `TEST_ASSERT_CONTENT_TYPE_IS`** to AST
3. **Update parser** to handle `contentType is` syntax
4. **Test framework integration** for explicit content-type checking

#### Long Term (Phase 3) - Advanced Features
1. **Additional assertion types** (contains, matches, etc.)
2. **Advanced comparison modes**
3. **Enhanced error messages** and debugging

## Summary

This plan provides a phased approach to fixing the current content-type issue while building toward comprehensive content assertion support. The key insight is that **Phase 1 can fix the immediate problem without any parser changes** by enhancing the testing framework's content detection and comparison logic.

**Immediate Fix:**
- Auto-detect content type from assertion content
- Use appropriate comparison method (JSON vs HTML/text)
- Normalize whitespace for HTML content
- Maintain full backward compatibility

**Future Enhancements:**
- Explicit `contentType is` assertions
- Advanced content matching (contains, patterns, etc.)
- Better error messages and debugging tools