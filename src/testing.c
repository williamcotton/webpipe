#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
#include <jansson.h>
#include "wp.h"

// Thread-local test context
static _Thread_local test_context_t *current_test_context = NULL;

// Global test mode state
static bool global_test_mode = false;

// Hash table implementation
hash_table_t *create_hash_table(MemoryArena *arena, int bucket_count) {
    hash_table_t *table = arena_alloc(arena, sizeof(hash_table_t));
    table->buckets = arena_alloc(arena, sizeof(hash_entry_t*) * bucket_count);
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
        hash = ((hash << 5) + hash) + c;
    }
    return hash % bucket_count;
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
                printf("Mock registered: %s%s%s\n", 
                       mock->data.mock_config.middleware_name,
                       mock->data.mock_config.variable_name ? "." : "",
                       mock->data.mock_config.variable_name ? mock->data.mock_config.variable_name : "");
            } else {
                fprintf(stderr, "Failed to parse mock JSON: %s\n", error.text);
            }
        }
    }
}

// Helper function to validate assertions
bool validate_assertions(ASTNode **assertions, int assertion_count, 
                        json_t *result, int status_code) {
    for (int i = 0; i < assertion_count; i++) {
        ASTNode *assertion = assertions[i];
        if (assertion->type == AST_TEST_ASSERTION) {
            switch (assertion->data.test_assertion.type) {
                case TEST_ASSERT_OUTPUT_EQUALS: {
                    json_error_t error;
                    json_t *expected = json_loads(assertion->data.test_assertion.data.output_equals.expected_json, 0, &error);
                    if (!expected) {
                        fprintf(stderr, "Failed to parse expected JSON: %s\n", error.text);
                        return false;
                    }
                    
                    if (!json_equal(result, expected)) {
                        char *result_str = json_dumps(result, JSON_INDENT(2));
                        char *expected_str = json_dumps(expected, JSON_INDENT(2));
                        printf("    Expected: %s\n", expected_str);
                        printf("    Got:      %s\n", result_str);
                        // Don't free these as they may be arena-allocated in some contexts
                        return false;
                    }
                    break;
                }
                case TEST_ASSERT_STATUS_IS: {
                    int expected_status = assertion->data.test_assertion.data.status_is.expected_status;
                    if (status_code != expected_status) {
                        printf("    Expected status: %d\n", expected_status);
                        printf("    Got status:      %d\n", status_code);
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
    // TODO: Implement variable test execution
    // This would find the variable in runtime->variables and execute it
    printf("Variable test execution not yet implemented\n");
    return json_object();
}

json_t *execute_pipeline_test(ASTNode *exec_node, test_context_t *ctx) {
    // TODO: Implement pipeline test execution
    // This would find the pipeline definition and execute it
    printf("Pipeline test execution not yet implemented\n");
    return json_object();
}

json_t *create_test_request_json(const char *method, const char *url, json_t *test_input) {
    json_t *request = json_object();
    
    // Basic HTTP request structure
    json_object_set_new(request, "method", json_string(method));
    json_object_set_new(request, "url", json_string(url));
    json_object_set_new(request, "params", json_object());  // Populated by match_route()
    json_object_set_new(request, "query", json_object());
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
                              MemoryArena *arena, int *status_code) {
    // If pipeline is empty, return the request object
    if (!route_stmt->data.route_def.pipeline) {
        *status_code = 200;
        return request;
    }
    
    // Execute pipeline with result handling (reuses server.c logic)
    json_t *final_response = NULL;
    int response_code = 200;
    char *content_type = NULL;
    
    int result = execute_pipeline_with_result(route_stmt->data.route_def.pipeline, 
                                            request, arena, &final_response, 
                                            &response_code, &content_type);
    
    if (result == 0 && final_response) {
        *status_code = response_code;
        return final_response;
    } else {
        // Error in pipeline execution
        *status_code = 500;
        return create_error_json("Internal server error");
    }
}

json_t *execute_route_test(ASTNode *exec_node, test_context_t *ctx, int *status_code) {
    // TODO: Need to access the runtime to find matching routes
    // This is a placeholder implementation
    printf("Route test execution not yet implemented\n");
    *status_code = 200;
    return json_object();
}

bool execute_it_block(ASTNode *it_node, test_context_t *ctx) {
    if (it_node->type != AST_IT_BLOCK) {
        return false;
    }
    
    // Execute the test
    json_t *result = NULL;
    int status_code = 200;
    
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
                result = execute_route_test(execution, ctx, &status_code);
                break;
        }
    }
    
    // Validate assertions
    bool success = validate_assertions(it_node->data.it_block.assertions, 
                                     it_node->data.it_block.assertion_count,
                                     result, status_code);
    
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
    
    printf("\n%s\n", describe_node->data.describe_block.description);
    
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
            printf("  ✓ %s\n", test->data.it_block.description);
        } else {
            printf("  ✗ %s\n", test->data.it_block.description);
        }
    }
    
    return 0;
}

int execute_test_suite(ASTNode *program) {
    printf("Executing test suite...\n");
    
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
    
    // Print results
    printf("\nTest Results: %d/%d passed\n", passed_tests, total_tests);
    
    arena_free(test_arena);
    return (passed_tests == total_tests) ? 0 : 1;
}