#ifndef TEST_UTILS_H
#define TEST_UTILS_H

#include "../../src/wp.h"
#include "../unity/unity.h"
#include <jansson.h>

// Test helper macros
#define TEST_ASSERT_NOT_NULL_MESSAGE(ptr, msg) \
    TEST_ASSERT_NOT_NULL_MESSAGE(ptr, msg)

#define TEST_ASSERT_JSON_EQUAL(expected, actual) \
    test_assert_json_equal(expected, actual, __LINE__)

#define TEST_ASSERT_STRING_EQUAL_OR_NULL(expected, actual) \
    test_assert_string_equal_or_null(expected, actual, __LINE__)

#define TEST_ASSERT_STRING_EQUAL(expected, actual) \
    TEST_ASSERT_EQUAL_STRING(expected, actual)

// Test utility functions
void test_assert_json_equal(json_t *expected, json_t *actual, int line);
void test_assert_string_equal_or_null(const char *expected, const char *actual, int line);

// Memory testing utilities
MemoryArena *create_test_arena(size_t size);
void destroy_test_arena(MemoryArena *arena);
void assert_arena_empty(MemoryArena *arena);
void assert_arena_used(MemoryArena *arena, size_t expected_used);

// JSON testing utilities
json_t *create_test_request(const char *method, const char *url);
json_t *create_test_request_with_params(const char *method, const char *url, json_t *params);
json_t *create_test_request_with_body(const char *method, const char *url, json_t *body);
json_t *parse_json_string(const char *json_str);
char *json_to_string(json_t *json);

// Lexer testing utilities
Token *tokenize_test_string(const char *source, int *token_count);
void free_test_tokens(Token *tokens, int count);
void assert_token_type(Token *token, TokenType expected_type);
void assert_token_value(Token *token, const char *expected_value);

// Parser testing utilities
ASTNode *parse_test_string(const char *source);
void free_test_ast(ASTNode *ast);
void assert_ast_type(ASTNode *node, ASTNodeType expected_type);

// Plugin testing utilities
Plugin *create_mock_plugin(const char *name, json_t *(*execute_func)(json_t *, void *, arena_alloc_func, arena_free_func, const char *));
void destroy_mock_plugin(Plugin *plugin);
json_t *mock_plugin_passthrough(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config);
json_t *mock_plugin_error(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config);

// HTTP testing utilities
struct test_http_response {
    int status_code;
    char *body;
    size_t body_size;
};

struct test_http_response *create_test_response(void);
void destroy_test_response(struct test_http_response *response);
int simulate_http_request(const char *method, const char *url, const char *body, struct test_http_response *response);

// Database testing utilities
void setup_test_database(void);
void teardown_test_database(void);
void create_test_tables(void);
void insert_test_data(void);
void clear_test_data(void);

// Performance testing utilities
void start_timer(void);
double end_timer(void);
void assert_execution_time_under(double max_seconds);

// Arena allocation wrapper for plugin interface
arena_alloc_func get_arena_alloc_wrapper(void);

// Error simulation utilities
void simulate_memory_shortage(void);
void restore_memory_functions(void);
void simulate_plugin_failure(const char *plugin_name);
void restore_plugin_functions(void);

#endif // TEST_UTILS_H