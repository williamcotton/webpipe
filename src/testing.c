#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
#include <ctype.h>
#include <jansson.h>
#include "wp.h"

// ANSI color codes for test output
#define ANSI_RESET   "\033[0m"
#define ANSI_BOLD    "\033[1m"
#define ANSI_GREEN   "\033[32m"
#define ANSI_RED     "\033[31m"
#define ANSI_YELLOW  "\033[33m"
#define ANSI_BLUE    "\033[34m"
#define ANSI_CYAN    "\033[36m"
#define ANSI_GRAY    "\033[90m"

// Unicode symbols
#define TEST_PASS_SYMBOL "✓"
#define TEST_FAIL_SYMBOL "✗"
#define TEST_SUITE_SYMBOL "📋"

// Thread-local test context
static _Thread_local test_context_t *current_test_context = NULL;

// Global test mode state
static bool global_test_mode = false;

// Hash table implementation
hash_table_t *create_hash_table(MemoryArena *arena, int bucket_count) {
    hash_table_t *table = arena_alloc(arena, sizeof(hash_table_t));
    table->buckets = arena_alloc(arena, sizeof(hash_entry_t*) * (size_t)bucket_count);
    table->bucket_count = bucket_count;
    table->arena = arena;
    
    // Initialize buckets to NULL
    for (int i = 0; i < bucket_count; i++) {
        table->buckets[i] = NULL;
    }
    
    return table;
}

unsigned int hash_string(const char *str, int bucket_count) {
    unsigned int hash = 5381;
    int c;
    while ((c = *str++)) {
        hash = ((hash << 5) + hash) + (unsigned int)c;
    }
    return hash % (unsigned int)bucket_count;
}

void hash_table_set(hash_table_t *table, const char *key, void *value) {
    unsigned int bucket = hash_string(key, table->bucket_count);
    
    hash_entry_t *entry = arena_alloc(table->arena, sizeof(hash_entry_t));
    entry->key = arena_strdup(table->arena, key);
    entry->value = value;
    entry->next = table->buckets[bucket];
    table->buckets[bucket] = entry;
}

void *hash_table_get(hash_table_t *table, const char *key) {
    unsigned int bucket = hash_string(key, table->bucket_count);
    
    hash_entry_t *entry = table->buckets[bucket];
    while (entry) {
        if (strcmp(entry->key, key) == 0) {
            return entry->value;
        }
        entry = entry->next;
    }
    
    return NULL;
}

// Test mode detection
bool is_test_mode_enabled(void) {
    return global_test_mode;
}

void set_test_mode(bool enabled) {
    global_test_mode = enabled;
}

// Test context management
test_context_t *create_test_context(MemoryArena *arena) {
    test_context_t *ctx = arena_alloc(arena, sizeof(test_context_t));
    ctx->middleware_mocks = create_hash_table(arena, 32);
    ctx->variable_mocks = create_hash_table(arena, 64);
    ctx->results = arena_alloc(arena, sizeof(test_results_t));
    ctx->test_arena = arena;
    ctx->is_test_mode = true;
    
    // Initialize results
    ctx->results->total_tests = 0;
    ctx->results->passed_tests = 0;
    ctx->results->failed_tests = 0;
    
    return ctx;
}

void set_test_context(test_context_t *ctx) {
    current_test_context = ctx;
}

test_context_t *get_test_context(void) {
    return current_test_context;
}

// Mock registry functions
void register_mock(test_context_t *ctx, const char *middleware_name, 
                   const char *variable_name, json_t *mock_data) {
    char key[256];
    
    if (variable_name) {
        // Variable-specific mock: "pg.teamsQuery"
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        hash_table_set(ctx->variable_mocks, key, mock_data);
    } else {
        // Middleware-wide mock: "pg"
        hash_table_set(ctx->middleware_mocks, middleware_name, mock_data);
    }
}

bool is_mock_active(test_context_t *ctx, const char *middleware_name, 
                    const char *variable_name) {
    if (!ctx) return false;
    
    char key[256];
    
    // Check for variable-specific mock first
    if (variable_name) {
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        if (hash_table_get(ctx->variable_mocks, key)) {
            return true;
        }
    }
    
    // Check for middleware-wide mock
    return hash_table_get(ctx->middleware_mocks, middleware_name) != NULL;
}

json_t *get_mock_result(test_context_t *ctx, const char *middleware_name, 
                        const char *variable_name) {
    char key[256];
    
    // Try variable-specific mock first
    if (variable_name) {
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        json_t *result = hash_table_get(ctx->variable_mocks, key);
        if (result) return result;
    }
    
    // Fall back to middleware-wide mock
    return hash_table_get(ctx->middleware_mocks, middleware_name);
}

// Helper function to check if program has test blocks
bool has_test_blocks(ASTNode *program) {
    if (!program || program->type != AST_PROGRAM) {
        return false;
    }
    
    for (int i = 0; i < program->data.program.statement_count; i++) {
        if (program->data.program.statements[i]->type == AST_DESCRIBE_BLOCK) {
            return true;
        }
    }
    
    return false;
}

// Helper function to apply mocks from AST nodes
static void apply_mocks(test_context_t *ctx, ASTNode **mock_configs, int mock_count) {
    for (int i = 0; i < mock_count; i++) {
        ASTNode *mock = mock_configs[i];
        if (mock->type == AST_MOCK_CONFIG) {
            // Parse the JSON return value
            json_error_t error;
            json_t *mock_data = json_loads(mock->data.mock_config.return_value, 0, &error);
            if (mock_data) {
                register_mock(ctx, 
                             mock->data.mock_config.middleware_name,
                             mock->data.mock_config.variable_name,
                             mock_data);
                printf("    %s📌 Mock registered:%s %s%s%s%s\n", 
                       ANSI_GRAY, ANSI_RESET, ANSI_CYAN,
                       mock->data.mock_config.middleware_name,
                       mock->data.mock_config.variable_name ? "." : "",
                       mock->data.mock_config.variable_name ? mock->data.mock_config.variable_name : "");
                printf("%s", ANSI_RESET);
            } else {
                fprintf(stderr, "Failed to parse mock JSON: %s\n", error.text);
            }
        }
    }
}

// Helper function to normalize whitespace for comparison
static char *normalize_whitespace(const char *str) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    char *normalized = malloc(len + 1);
    char *dst = normalized;
    bool in_whitespace = false;
    
    // Trim leading whitespace
    while (*str && isspace((unsigned char)*str)) str++;
    
    while (*str) {
        if (isspace((unsigned char)*str)) {
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
    while (dst > normalized && isspace((unsigned char)*(dst-1))) dst--;
    
    *dst = '\0';
    return normalized;
}

// Helper function to extract content from result based on content-type
static const char *extract_content_from_result(json_t *result, const char *content_type) {
    if (!result) return NULL;
    
    // If content-type indicates HTML or text, extract string value directly
    if (content_type && (strstr(content_type, "text/html") || 
                        strstr(content_type, "text/plain") ||
                        strstr(content_type, "text/css") ||
                        strstr(content_type, "text/javascript"))) {
        if (json_is_string(result)) {
            return json_string_value(result);  // Don't dup, just return pointer
        }
    }
    
    // For JSON or when content-type is not available, serialize as JSON
    // Note: json_dumps returns arena-allocated memory in test context
    return json_dumps(result, JSON_COMPACT);
}

// Helper function to detect if content looks like JSON
static bool is_json_content(const char *content) {
    if (!content) return false;
    
    // Skip leading whitespace
    while (isspace((unsigned char)*content)) content++;
    
    // Check if it starts with JSON structure
    return (*content == '{' || *content == '[');
}

// Helper function to validate assertions
bool validate_assertions(ASTNode **assertions, int assertion_count, 
                        json_t *result, int status_code, const char *content_type) {
    for (int i = 0; i < assertion_count; i++) {
        ASTNode *assertion = assertions[i];
        if (assertion->type == AST_TEST_ASSERTION) {
            switch (assertion->data.test_assertion.type) {
                case TEST_ASSERT_OUTPUT_EQUALS: {
                    const char *expected_content = assertion->data.test_assertion.data.output_equals.expected_json;
                    const char *actual_content = extract_content_from_result(result, content_type);
                    
                    // Auto-detect comparison mode based on content and content-type
                    if (is_json_content(expected_content) && 
                        (!content_type || strstr(content_type, "application/json"))) {
                        // JSON comparison mode
                        json_error_t error;
                        json_t *expected = json_loads(expected_content, 0, &error);
                        if (!expected) {
                            printf("    %s❌ JSON parsing failed:%s %s\n", ANSI_RED, ANSI_RESET, error.text);
                            return false;
                        }
                        
                        if (!json_equal(result, expected)) {
                            const char *expected_str = json_dumps(expected, JSON_INDENT(2));
                            printf("    %s❌ JSON assertion failed:%s\n", ANSI_RED, ANSI_RESET);
                            printf("    %sExpected:%s %s\n", ANSI_YELLOW, ANSI_RESET, expected_str);
                            printf("    %sGot:     %s %s\n", ANSI_YELLOW, ANSI_RESET, actual_content);
                            return false;
                        }
                    } else {
                        // Text/HTML comparison mode with whitespace normalization
                        char *norm_expected = normalize_whitespace(expected_content);
                        char *norm_actual = normalize_whitespace(actual_content);
                        
                        if (strcmp(norm_expected, norm_actual) != 0) {
                            printf("    %s❌ Content assertion failed:%s\n", ANSI_RED, ANSI_RESET);
                            printf("    %sContent-Type:%s %s\n", ANSI_BLUE, ANSI_RESET, 
                                   content_type ? content_type : "unknown");
                            printf("    %sExpected:%s %s\n", ANSI_YELLOW, ANSI_RESET, expected_content);
                            printf("    %sGot:     %s %s\n", ANSI_YELLOW, ANSI_RESET, actual_content);
                            if (norm_expected) free(norm_expected);
                            if (norm_actual) free(norm_actual);
                            return false;
                        }
                        
                        if (norm_expected) free(norm_expected);
                        if (norm_actual) free(norm_actual);
                    }
                    
                    break;
                }
                case TEST_ASSERT_STATUS_IS: {
                    int expected_status = assertion->data.test_assertion.data.status_is.expected_status;
                    if (status_code != expected_status) {
                        printf("    %s❌ Status assertion failed:%s\n", ANSI_RED, ANSI_RESET);
                        printf("    %sExpected status:%s %d\n", ANSI_YELLOW, ANSI_RESET, expected_status);
                        printf("    %sGot status:     %s %d\n", ANSI_YELLOW, ANSI_RESET, status_code);
                        return false;
                    }
                    break;
                }
            }
        }
    }
    return true;
}

// Helper function to create error JSON
json_t *create_error_json(const char *message) {
    json_t *error = json_object();
    json_object_set_new(error, "error", json_string(message));
    return error;
}

// Test execution functions
json_t *execute_variable_test(ASTNode *exec_node, test_context_t *ctx) {
    if (exec_node->type != AST_TEST_EXECUTION || 
        exec_node->data.test_execution.type != TEST_EXEC_VARIABLE) {
        fprintf(stderr, "Invalid variable test execution node\n");
        return json_object();
    }

    char *middleware_type = exec_node->data.test_execution.data.variable.middleware_type;
    char *variable_name = exec_node->data.test_execution.data.variable.variable_name;
    char *input_json = exec_node->data.test_execution.data.variable.input_json;

    // Look up the variable in runtime->variables
    json_t *variable = json_object_get(runtime->variables, variable_name);
    if (!variable) {
        fprintf(stderr, "Variable not found: %s\n", variable_name);
        return json_object();
    }

    // Find the middleware
    Middleware *mw = find_middleware(middleware_type);
    if (!mw) {
        fprintf(stderr, "Middleware not found: %s\n", middleware_type);
        return json_object();
    }

    // Parse input JSON if provided
    json_t *input = json_object();
    if (input_json) {
        json_error_t error;
        json_t *parsed_input = json_loads(input_json, 0, &error);
        if (parsed_input) {
            input = parsed_input;
        } else {
            fprintf(stderr, "Failed to parse input JSON: %s\n", error.text);
        }
    }

    // Execute the variable through pipeline execution to get mock interception
    // Create a single-step pipeline for the variable
    PipelineStep *step = arena_alloc(ctx->test_arena, sizeof(PipelineStep));
    step->middleware = arena_strdup(ctx->test_arena, middleware_type);
    step->value = arena_strdup(ctx->test_arena, variable_name); 
    step->is_variable = true;  // Mark as variable reference
    step->next = NULL;

    // Execute through pipeline system to get mock interception
    json_t *result = NULL;
    int response_code = 200;
    char *content_type = NULL;
    int ok = execute_pipeline_with_result(step, input, ctx->test_arena, 
                                         &result, &response_code, &content_type);

    return (ok == 0 && result) ? result : json_object();
}

json_t *execute_pipeline_test(ASTNode *exec_node, test_context_t *ctx) {
    if (exec_node->type != AST_TEST_EXECUTION || 
        exec_node->data.test_execution.type != TEST_EXEC_PIPELINE) {
        fprintf(stderr, "Invalid pipeline test execution node\n");
        return json_object();
    }

    char *pipeline_name = exec_node->data.test_execution.data.pipeline.pipeline_name;
    char *input_json = exec_node->data.test_execution.data.pipeline.input_json;

    // Look up the pipeline definition in runtime->variables
    json_t *pipeline_var = json_object_get(runtime->variables, pipeline_name);
    if (!pipeline_var) {
        fprintf(stderr, "Pipeline not found: %s\n", pipeline_name);
        return json_object();
    }

    // Verify it's a pipeline definition
    json_t *type_field = json_object_get(pipeline_var, "_type");
    if (!json_is_string(type_field) || 
        strcmp(json_string_value(type_field), "pipeline") != 0) {
        fprintf(stderr, "Variable '%s' is not a pipeline definition\n", pipeline_name);
        return json_object();
    }

    // Get pipeline AST node
    ASTNode *pnode = (ASTNode *)(uintptr_t)json_integer_value(
        json_object_get(pipeline_var, "_definition"));
    if (!pnode || pnode->type != AST_PIPELINE_DEFINITION) {
        fprintf(stderr, "Corrupted pipeline definition for '%s'\n", pipeline_name);
        return json_object();
    }

    // Parse input JSON if provided, otherwise use empty object
    json_t *input = json_object();
    if (input_json) {
        json_error_t error;
        json_t *parsed_input = json_loads(input_json, 0, &error);
        if (parsed_input) {
            input = parsed_input;
        } else {
            fprintf(stderr, "Failed to parse input JSON: %s\n", error.text);
        }
    }

    // Execute pipeline with mock interception through existing system
    json_t *result = NULL;
    int response_code = 200;
    char *content_type = NULL;
    int ok = execute_pipeline_with_result(pnode->data.pipeline_def.pipeline, input,
                                         ctx->test_arena, &result, &response_code, &content_type);

    return (ok == 0 && result) ? result : json_object();
}

json_t *create_test_request_json(const char *method, const char *url, json_t *test_input) {
    json_t *request = json_object();
    
    // Basic HTTP request structure
    json_object_set_new(request, "method", json_string(method));
    json_object_set_new(request, "url", json_string(url));
    json_object_set_new(request, "params", json_object());  // Populated by match_route()
    
    // Parse query parameters from URL
    json_t *query = json_object();
    char *query_start = strchr(url, '?');
    if (query_start) {
        query_start++; // Skip the '?'
        char *query_copy = strdup(query_start);
        char *saveptr = NULL;
        char *pair = strtok_r(query_copy, "&", &saveptr);
        
        while (pair) {
            char *equals = strchr(pair, '=');
            if (equals) {
                *equals = '\0'; // Split key=value
                char *key = pair;
                char *value = equals + 1;
                json_object_set_new(query, key, json_string(value));
            }
            pair = strtok_r(NULL, "&", &saveptr);
        }
        
        free(query_copy);
    }
    json_object_set_new(request, "query", query);
    
    json_object_set_new(request, "headers", json_object());
    json_object_set_new(request, "cookies", json_object());
    json_object_set_new(request, "setCookies", json_array());
    
    // Use test input as body if provided, otherwise null
    if (test_input) {
        json_object_set(request, "body", test_input);
    } else {
        json_object_set_new(request, "body", json_null());
    }
    
    return request;
}

json_t *execute_route_pipeline(ASTNode *route_stmt, json_t *request, 
                              MemoryArena *arena, int *status_code, char **content_type) {
    // If pipeline is empty, return the request object
    if (!route_stmt->data.route_def.pipeline) {
        *status_code = 200;
        *content_type = NULL;
        return request;
    }
    
    // Execute pipeline with result handling (reuses server.c logic)
    json_t *final_response = NULL;
    int response_code = 200;
    char *pipeline_content_type = NULL;
    
    int result = execute_pipeline_with_result(route_stmt->data.route_def.pipeline, 
                                            request, arena, &final_response, 
                                            &response_code, &pipeline_content_type);
    
    if (result == 0 && final_response) {
        *status_code = response_code;
        *content_type = pipeline_content_type;
        // Clean up response to match production behavior
        return cleanup_response_json(final_response);
    } else {
        // Error in pipeline execution
        *status_code = 500;
        *content_type = NULL;
        return create_error_json("Internal server error");
    }
}

json_t *execute_route_test(ASTNode *exec_node, test_context_t *ctx, int *status_code, char **content_type) {
    if (exec_node->type != AST_TEST_EXECUTION || 
        exec_node->data.test_execution.type != TEST_EXEC_HTTP_CALL) {
        fprintf(stderr, "Invalid route test execution node\n");
        *status_code = 500;
        *content_type = NULL;
        return json_object();
    }

    const char *method = exec_node->data.test_execution.data.http_call.method;
    const char *url = exec_node->data.test_execution.data.http_call.path;

    // Create synthetic request object (no real HTTP connection needed)
    json_t *request = create_test_request_json(method, url, NULL);
    
    // Set test arena context
    set_current_arena(ctx->test_arena);
    
    // Find matching route using existing server.c logic
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_ROUTE_DEFINITION) {
            if (strcmp(stmt->data.route_def.method, method) == 0) {
                json_t *params = json_object_get(request, "params");
                // Reuse existing match_route() function
                if (match_route(stmt->data.route_def.route, url, params)) {
                    // Execute route pipeline directly (bypassing HTTP layer)
                    return execute_route_pipeline(stmt, request, ctx->test_arena, status_code, content_type);
                }
            }
        }
    }
    
    // Route not found
    *status_code = 404;
    *content_type = NULL;
    return create_error_json("Route not found");
}

bool execute_it_block(ASTNode *it_node, test_context_t *ctx) {
    if (it_node->type != AST_IT_BLOCK) {
        return false;
    }
    
    // Apply inline mocks (these override describe-level mocks)
    if (it_node->data.it_block.inline_mocks && it_node->data.it_block.inline_mock_count > 0) {
        apply_mocks(ctx, it_node->data.it_block.inline_mocks, it_node->data.it_block.inline_mock_count);
    }
    
    // Execute the test
    json_t *result = NULL;
    int status_code = 200;
    char *content_type = NULL;
    
    ASTNode *execution = it_node->data.it_block.execution;
    if (execution && execution->type == AST_TEST_EXECUTION) {
        switch (execution->data.test_execution.type) {
            case TEST_EXEC_VARIABLE:
                result = execute_variable_test(execution, ctx);
                break;
            case TEST_EXEC_PIPELINE:
                result = execute_pipeline_test(execution, ctx);
                break;
            case TEST_EXEC_HTTP_CALL:
                result = execute_route_test(execution, ctx, &status_code, &content_type);
                break;
        }
    }
    
    // Validate assertions
    bool success = validate_assertions(it_node->data.it_block.assertions, 
                                     it_node->data.it_block.assertion_count,
                                     result, status_code, content_type);
    
    // Update test results
    ctx->results->total_tests++;
    if (success) {
        ctx->results->passed_tests++;
    } else {
        ctx->results->failed_tests++;
    }
    
    return success;
}

int execute_describe_block(ASTNode *describe_node, test_context_t *ctx, 
                          int *total, int *passed) {
    if (describe_node->type != AST_DESCRIBE_BLOCK) {
        return -1;
    }
    
    printf("\n%s%s%s%s\n", ANSI_BOLD, ANSI_BLUE, describe_node->data.describe_block.description, ANSI_RESET);
    
    *total = 0;
    *passed = 0;
    
    // Set up describe-level mocks
    apply_mocks(ctx, describe_node->data.describe_block.mock_configs, 
                describe_node->data.describe_block.mock_count);
    
    // Execute each test case
    for (int i = 0; i < describe_node->data.describe_block.test_count; i++) {
        ASTNode *test = describe_node->data.describe_block.tests[i];
        (*total)++;
        if (execute_it_block(test, ctx)) {
            (*passed)++;
            printf("  %s%s%s %s%s\n", ANSI_GREEN, TEST_PASS_SYMBOL, ANSI_RESET, 
                   test->data.it_block.description, ANSI_RESET);
        } else {
            printf("  %s%s%s %s%s\n", ANSI_RED, TEST_FAIL_SYMBOL, ANSI_RESET,
                   test->data.it_block.description, ANSI_RESET);
        }
    }
    
    return 0;
}

int execute_test_suite(ASTNode *program) {
    printf("\n%s%s %s Test Suite%s\n", ANSI_BOLD, TEST_SUITE_SYMBOL, ANSI_CYAN, ANSI_RESET);
    printf("%s%s%s\n", ANSI_GRAY, "====================", ANSI_RESET);
    
    MemoryArena *test_arena = arena_create(1024 * 1024 * 10); // 10MB for tests
    test_context_t *test_ctx = create_test_context(test_arena);
    set_test_context(test_ctx);
    
    int total_tests = 0;
    int passed_tests = 0;
    
    // Find and execute all describe blocks
    for (int i = 0; i < program->data.program.statement_count; i++) {
        ASTNode *stmt = program->data.program.statements[i];
        if (stmt->type == AST_DESCRIBE_BLOCK) {
            int suite_total, suite_passed;
            execute_describe_block(stmt, test_ctx, &suite_total, &suite_passed);
            total_tests += suite_total;
            passed_tests += suite_passed;
        }
    }
    
    // Print results with color coding
    printf("\n%s%s%s\n", ANSI_GRAY, "====================", ANSI_RESET);
    if (passed_tests == total_tests) {
        printf("%s%s Test Results: %s%d/%d passed%s\n", 
               ANSI_BOLD, ANSI_GREEN, ANSI_GREEN, passed_tests, total_tests, ANSI_RESET);
    } else {
        printf("%s%s Test Results: %s%d/%d passed%s (%s%d failed%s)\n", 
               ANSI_BOLD, ANSI_YELLOW, ANSI_YELLOW, passed_tests, total_tests, ANSI_RESET,
               ANSI_RED, total_tests - passed_tests, ANSI_RESET);
    }
    printf("%s%s%s\n", ANSI_GRAY, "====================", ANSI_RESET);
    
    arena_free(test_arena);
    return (passed_tests == total_tests) ? 0 : 1;
}
