#include "test_utils.h"
#include <string.h>
#include <time.h>
#include <sys/time.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <microhttpd.h>

// Arena allocation wrapper for middleware interface
static void *arena_alloc_wrapper(void *arena, size_t size) {
    return arena_alloc((MemoryArena *)arena, size);
}

static struct timeval start_time;

void test_assert_json_equal(json_t *expected, json_t *actual, int line) {
    if (expected == NULL && actual == NULL) {
        return;
    }
    
    if (expected == NULL || actual == NULL) {
        char msg[256];
        snprintf(msg, sizeof(msg), "JSON comparison failed at line %d: one is NULL", line);
        TEST_FAIL_MESSAGE(msg);
    }
    
    if (!json_equal(expected, actual)) {
        char *expected_str = json_dumps(expected, JSON_INDENT(2));
        char *actual_str = json_dumps(actual, JSON_INDENT(2));
        
        char msg[1024];
        snprintf(msg, sizeof(msg), 
            "JSON comparison failed at line %d:\nExpected:\n%s\nActual:\n%s", 
            line, expected_str, actual_str);
        
        free(expected_str);
        free(actual_str);
        TEST_FAIL_MESSAGE(msg);
    }
}

void test_assert_string_equal_or_null(const char *expected, const char *actual, int line) {
    if (expected == NULL && actual == NULL) {
        return;
    }
    
    if (expected == NULL || actual == NULL) {
        char msg[256];
        snprintf(msg, sizeof(msg), "String comparison failed at line %d: one is NULL", line);
        TEST_FAIL_MESSAGE(msg);
    }
    
    if (strcmp(expected, actual) != 0) {
        char msg[512];
        snprintf(msg, sizeof(msg), 
            "String comparison failed at line %d:\nExpected: '%s'\nActual: '%s'", 
            line, expected, actual);
        TEST_FAIL_MESSAGE(msg);
    }
}

MemoryArena *create_test_arena(size_t size) {
    return arena_create(size);
}

void destroy_test_arena(MemoryArena *arena) {
    arena_free(arena);
}

void assert_arena_empty(MemoryArena *arena) {
    TEST_ASSERT_EQUAL(0, arena->used);
}

void assert_arena_used(MemoryArena *arena, size_t expected_used) {
    TEST_ASSERT_EQUAL(expected_used, arena->used);
}

json_t *create_test_request(const char *method, const char *url) {
    json_t *request = json_object();
    json_object_set_new(request, "method", json_string(method));
    json_object_set_new(request, "url", json_string(url));
    json_object_set_new(request, "query", json_object());
    json_object_set_new(request, "params", json_object());
    json_object_set_new(request, "headers", json_object());
    json_object_set_new(request, "body", json_object());
    return request;
}

json_t *create_test_request_with_params(const char *method, const char *url, json_t *params) {
    json_t *request = create_test_request(method, url);
    json_object_set(request, "params", params);
    return request;
}

json_t *create_test_request_with_body(const char *method, const char *url, json_t *body) {
    json_t *request = create_test_request(method, url);
    json_object_set(request, "body", body);
    return request;
}

json_t *parse_json_string(const char *json_str) {
    json_error_t error;
    json_t *json = json_loads(json_str, 0, &error);
    if (!json) {
        char msg[512];
        snprintf(msg, sizeof(msg), "Failed to parse JSON: %s", error.text);
        TEST_FAIL_MESSAGE(msg);
    }
    return json;
}

char *json_to_string(json_t *json) {
    return json_dumps(json, JSON_COMPACT);
}

Token *tokenize_test_string(const char *source, int *token_count) {
    return lexer_tokenize(source, token_count);
}

void free_test_tokens(Token *tokens, int count) {
    free_tokens(tokens, count);
}

void assert_token_type(Token *token, TokenType expected_type) {
    TEST_ASSERT_EQUAL(expected_type, token->type);
}

void assert_token_value(Token *token, const char *expected_value) {
    TEST_ASSERT_STRING_EQUAL_OR_NULL(expected_value, token->value);
}

ASTNode *parse_test_string(const char *source) {
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);
    
    Parser *parser = parser_new(tokens, token_count);
    ASTNode *ast = parser_parse(parser);
    
    parser_free(parser);
    free_tokens(tokens, token_count);
    
    return ast;
}

void free_test_ast(ASTNode *ast) {
    free_ast(ast);
}

void assert_ast_type(ASTNode *node, ASTNodeType expected_type) {
    TEST_ASSERT_EQUAL(expected_type, node->type);
}

Middleware *create_mock_middleware(const char *name, json_t *(*execute_func)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *)) {
    Middleware *middleware = malloc(sizeof(Middleware));
    middleware->name = strdup(name);
    middleware->handle = NULL;
    middleware->execute = execute_func;
    return middleware;
}

void destroy_mock_middleware(Middleware *middleware) {
    if (middleware) {
        free(middleware->name);
        free(middleware);
    }
}

json_t *mock_middleware_passthrough(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, char **contentType, json_t *variables) {
    (void)arena;
    (void)alloc_func;
    (void)free_func;
    (void)config;
    (void)contentType;  // Mock middleware doesn't change content type
    (void)variables;    // Mock middleware doesn't use variables
    return json_incref(input);
}

json_t *mock_middleware_error(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, char **contentType, json_t *variables) {
    (void)input;
    (void)arena;
    (void)alloc_func;
    (void)free_func;
    (void)config;
    (void)variables;
    (void)contentType;  // Mock middleware doesn't change content type
    json_t *error_obj = json_object();
    json_t *errors = json_array();
    json_t *error = json_object();
    
    json_object_set_new(error, "type", json_string("mockError"));
    json_object_set_new(error, "message", json_string("Mock middleware error"));
    json_array_append_new(errors, error);
    json_object_set_new(error_obj, "errors", errors);
    
    return error_obj;
}

struct test_http_response *create_test_response(void) {
    struct test_http_response *response = malloc(sizeof(struct test_http_response));
    response->status_code = 0;
    response->body = NULL;
    response->body_size = 0;
    return response;
}

void destroy_test_response(struct test_http_response *response) {
    if (response) {
        free(response->body);
        free(response);
    }
}

void start_timer(void) {
    gettimeofday(&start_time, NULL);
}

double end_timer(void) {
    struct timeval end_time;
    gettimeofday(&end_time, NULL);
    
    double start_seconds = start_time.tv_sec + start_time.tv_usec / 1000000.0;
    double end_seconds = end_time.tv_sec + end_time.tv_usec / 1000000.0;
    
    return end_seconds - start_seconds;
}

void assert_execution_time_under(double max_seconds) {
    double elapsed = end_timer();
    if (elapsed > max_seconds) {
        char msg[256];
        snprintf(msg, sizeof(msg), "Execution took %.3f seconds, expected under %.3f seconds", elapsed, max_seconds);
        TEST_FAIL_MESSAGE(msg);
    }
}

arena_alloc_func get_arena_alloc_wrapper(void) {
    return arena_alloc_wrapper;
}

// Database testing utilities - placeholder implementations
void setup_test_database(void) {
    // TODO: Implement test database setup
}

void teardown_test_database(void) {
    // TODO: Implement test database teardown
}

void create_test_tables(void) {
    // TODO: Implement test table creation
}

void insert_test_data(void) {
    // TODO: Implement test data insertion
}

void clear_test_data(void) {
    // TODO: Implement test data clearing
}

// HTTP testing utilities - placeholder implementations
int simulate_http_request(const char *method, const char *url, const char *body __attribute__((unused)), struct test_http_response *response) {
    // Initialize test runtime if not already done
    if (!test_runtime) {
        if (init_test_runtime("test.wp") != 0) {
            return -1;
        }
    }
    
    // Create request JSON
    json_t *request = create_request_json(NULL, url, method, NULL);
    if (!request) {
        return -1;
    }
    
    // Create memory arena for request processing
    MemoryArena *arena = arena_create(1024 * 1024);
    if (!arena) {
        json_decref(request);
        return -1;
    }
    
    // Default response
    response->status_code = 200;
    response->body = strdup("{\"message\":\"Test response\"}");
    response->body_size = strlen(response->body);
    
    // Clean up
    json_decref(request);
    arena_free(arena);
    
    return 0;
}

// Error simulation utilities - placeholder implementations
void simulate_memory_shortage(void) {
    // TODO: Implement memory shortage simulation
}

void restore_memory_functions(void) {
    // TODO: Implement memory function restoration
}

void simulate_middleware_failure(const char *middleware_name __attribute__((unused))) {
    // TODO: Implement middleware failure simulation
}

void restore_middleware_functions(void) {
    // TODO: Implement middleware function restoration
}

// Compatible runtime structure definition for tests
typedef struct WPRuntime {
    struct MHD_Daemon *daemon;
    ASTNode *program;
    Middleware *middlewares;
    int middleware_count;
    json_t *variables;
    ParseContext *parse_ctx;
} WPRuntime;

// Test-safe runtime initialization without jansson custom allocators
struct WPRuntime *test_runtime = NULL; // Local test runtime

int init_test_runtime(const char *wp_file) {
    // Check if already initialized
    if (test_runtime) {
        return 0;
    }
    
    // Allocate runtime
    test_runtime = malloc(sizeof(struct WPRuntime));
    if (!test_runtime) {
        return -1;
    }
    
    // Initialize fields
    test_runtime->daemon = NULL;
    test_runtime->program = NULL;
    test_runtime->middlewares = NULL;
    test_runtime->middleware_count = 0;
    test_runtime->variables = json_object();  // Use default jansson allocators
    test_runtime->parse_ctx = NULL;
    
    // Parse the WP file for middleware loading without setting up HTTP server
    FILE *file = fopen(wp_file, "r");
    if (!file) {
        fprintf(stderr, "Error: Could not open file '%s'\n", wp_file);
        free(test_runtime);
        test_runtime = NULL;
        return -1;
    }
    
    // Read file content
    fseek(file, 0, SEEK_END);
    long file_size_long = ftell(file);
    if (file_size_long < 0) {
        fclose(file);
        free(test_runtime);
        test_runtime = NULL;
        return -1;
    }
    size_t file_size = (size_t)file_size_long;
    fseek(file, 0, SEEK_SET);
    
    char *source = malloc(file_size + 1);
    fread(source, 1, file_size, file);
    source[file_size] = '\0';
    fclose(file);
    
    // Tokenize and parse
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);
    if (!tokens) {
        free(source);
        free(test_runtime);
        test_runtime = NULL;
        return -1;
    }
    
    Parser *parser = parser_new(tokens, token_count);
    test_runtime->program = parser_parse(parser);
    
    if (!test_runtime->program) {
        free_tokens(tokens, token_count);
        parser_free(parser);
        free(source);
        free(test_runtime);
        test_runtime = NULL;
        return -1;
    }
    
    // Load middlewares without custom allocators - skip for now to avoid circular dependency
    // char *middleware_names[64];
    // int middleware_count = 0;
    // collect_middleware_names_from_ast(test_runtime->program, middleware_names, &middleware_count, 64);
    
    // for (int i = 0; i < middleware_count; i++) {
    //     if (load_middleware(middleware_names[i]) != 0) {
    //         fprintf(stderr, "Warning: Failed to load %s middleware\n", middleware_names[i]);
    //     }
    // }
    
    // Cleanup parsing resources
    free_tokens(tokens, token_count);
    parser_free(parser);
    free(source);
    
    return 0;
}

void cleanup_test_runtime(void) {
    if (test_runtime) {
        // Stop HTTP server if running
        if (test_runtime->daemon) {
            MHD_stop_daemon(test_runtime->daemon);
        }
        
        // Cleanup middlewares
        for (int i = 0; i < test_runtime->middleware_count; i++) {
            dlclose(test_runtime->middlewares[i].handle);
            free(test_runtime->middlewares[i].name);
        }
        free(test_runtime->middlewares);
        
        // Cleanup other resources
        if (test_runtime->variables) {
            json_decref(test_runtime->variables);
        }
        if (test_runtime->program) {
            free_ast(test_runtime->program);
        }
        
        free(test_runtime);
        test_runtime = NULL;
    }
}

int get_test_port(void) {
    return TEST_PORT;
}
