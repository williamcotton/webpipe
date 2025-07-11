#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

static void test_lexer_new_and_free(void) {
    const char *source = "GET /test";
    Lexer *lexer = lexer_new(source);
    
    TEST_ASSERT_NOT_NULL(lexer);
    TEST_ASSERT_EQUAL(source, lexer->source);
    TEST_ASSERT_EQUAL(0, lexer->current);
    TEST_ASSERT_EQUAL(1, lexer->line);
    TEST_ASSERT_EQUAL(1, lexer->column);
    
    lexer_free(lexer);
}

static void test_lexer_peek_and_advance(void) {
    const char *source = "GET";
    Lexer *lexer = lexer_new(source);
    
    TEST_ASSERT_EQUAL('G', lexer_peek(lexer));
    TEST_ASSERT_EQUAL(0, lexer->current);
    
    TEST_ASSERT_EQUAL('G', lexer_advance(lexer));
    TEST_ASSERT_EQUAL(1, lexer->current);
    TEST_ASSERT_EQUAL(1, lexer->line);
    TEST_ASSERT_EQUAL(2, lexer->column);
    
    TEST_ASSERT_EQUAL('E', lexer_peek(lexer));
    TEST_ASSERT_EQUAL('E', lexer_advance(lexer));
    TEST_ASSERT_EQUAL('T', lexer_advance(lexer));
    TEST_ASSERT_EQUAL('\0', lexer_advance(lexer));
    
    lexer_free(lexer);
}

static void test_lexer_skip_whitespace(void) {
    const char *source = "  \t  GET";
    Lexer *lexer = lexer_new(source);
    
    lexer_skip_whitespace(lexer);
    TEST_ASSERT_EQUAL('G', lexer_peek(lexer));
    TEST_ASSERT_EQUAL(5, lexer->current);
    
    lexer_free(lexer);
}

static void test_lexer_skip_whitespace_with_newlines(void) {
    const char *source = "  \n  \t\n  GET";
    Lexer *lexer = lexer_new(source);
    
    lexer_skip_whitespace(lexer);
    TEST_ASSERT_EQUAL('G', lexer_peek(lexer));
    TEST_ASSERT_EQUAL(3, lexer->line);
    TEST_ASSERT_EQUAL(3, lexer->column);
    
    lexer_free(lexer);
}

static void test_lexer_tokenize_http_methods(void) {
    const char *methods[] = {"GET", "POST", "PUT", "DELETE", "PATCH"};
    int num_methods = sizeof(methods) / sizeof(methods[0]);
    
    for (int i = 0; i < num_methods; i++) {
        int token_count;
        Token *tokens = tokenize_test_string(methods[i], &token_count);
        
        TEST_ASSERT_EQUAL(2, token_count);  // HTTP_METHOD + EOF
        assert_token_type(&tokens[0], TOKEN_HTTP_METHOD);
        assert_token_value(&tokens[0], methods[i]);
        assert_token_type(&tokens[1], TOKEN_EOF);
        
        free_test_tokens(tokens, token_count);
    }
}

static void test_lexer_tokenize_route_patterns(void) {
    const char *routes[] = {
        "/test",
        "/page/:id",
        "/user/:userId/post/:postId",
        "/api/v1/users"
    };
    int num_routes = sizeof(routes) / sizeof(routes[0]);
    
    for (int i = 0; i < num_routes; i++) {
        int token_count;
        Token *tokens = tokenize_test_string(routes[i], &token_count);
        
        TEST_ASSERT_GREATER_THAN(1, token_count);
        assert_token_type(&tokens[0], TOKEN_ROUTE);
        assert_token_value(&tokens[0], routes[i]);
        assert_token_type(&tokens[token_count - 1], TOKEN_EOF);
        
        free_test_tokens(tokens, token_count);
    }
}

static void test_lexer_tokenize_pipeline_operator(void) {
    const char *source = "|>";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(2, token_count);
    assert_token_type(&tokens[0], TOKEN_PIPE);
    assert_token_value(&tokens[0], "|>");
    assert_token_type(&tokens[1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_string_literals(void) {
    const char *source = "`{ id: .params.id }`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(2, token_count);
    assert_token_type(&tokens[0], TOKEN_STRING);
    assert_token_value(&tokens[0], "{ id: .params.id }");
    assert_token_type(&tokens[1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_multiline_string(void) {
    const char *source = "`{\n  id: .params.id,\n  name: .body.name\n}`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(2, token_count);
    assert_token_type(&tokens[0], TOKEN_STRING);
    assert_token_value(&tokens[0], "{\n  id: .params.id,\n  name: .body.name\n}");
    assert_token_type(&tokens[1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_identifiers(void) {
    const char *identifiers[] = {"jq", "lua", "pg", "result", "teamsQuery", "validateUser"};
    int num_identifiers = sizeof(identifiers) / sizeof(identifiers[0]);
    
    for (int i = 0; i < num_identifiers; i++) {
        int token_count;
        Token *tokens = tokenize_test_string(identifiers[i], &token_count);
        
        TEST_ASSERT_EQUAL(2, token_count);
        assert_token_type(&tokens[0], TOKEN_IDENTIFIER);
        assert_token_value(&tokens[0], identifiers[i]);
        assert_token_type(&tokens[1], TOKEN_EOF);
        
        free_test_tokens(tokens, token_count);
    }
}

static void test_lexer_tokenize_numbers(void) {
    const char *numbers[] = {"123", "200", "404", "500", "0"};
    int num_numbers = sizeof(numbers) / sizeof(numbers[0]);
    
    for (int i = 0; i < num_numbers; i++) {
        int token_count;
        Token *tokens = tokenize_test_string(numbers[i], &token_count);
        
        TEST_ASSERT_EQUAL(2, token_count);
        assert_token_type(&tokens[0], TOKEN_NUMBER);
        assert_token_value(&tokens[0], numbers[i]);
        assert_token_type(&tokens[1], TOKEN_EOF);
        
        free_test_tokens(tokens, token_count);
    }
}

static void test_lexer_tokenize_punctuation(void) {
    const char *source = "(): {}=";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(7, token_count);
    assert_token_type(&tokens[0], TOKEN_LPAREN);
    assert_token_type(&tokens[1], TOKEN_RPAREN);
    assert_token_type(&tokens[2], TOKEN_COLON);
    assert_token_type(&tokens[3], TOKEN_LBRACE);
    assert_token_type(&tokens[4], TOKEN_RBRACE);
    assert_token_type(&tokens[5], TOKEN_EQUALS);
    assert_token_type(&tokens[6], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_simple_route(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_GREATER_THAN(6, token_count);
    
    assert_token_type(&tokens[0], TOKEN_HTTP_METHOD);
    assert_token_value(&tokens[0], "GET");
    
    assert_token_type(&tokens[1], TOKEN_ROUTE);
    assert_token_value(&tokens[1], "/test");
    
    assert_token_type(&tokens[2], TOKEN_NEWLINE);
    
    assert_token_type(&tokens[3], TOKEN_PIPE);
    assert_token_value(&tokens[3], "|>");
    
    assert_token_type(&tokens[4], TOKEN_IDENTIFIER);
    assert_token_value(&tokens[4], "jq");
    
    assert_token_type(&tokens[5], TOKEN_COLON);
    
    assert_token_type(&tokens[6], TOKEN_STRING);
    assert_token_value(&tokens[6], "{ message: \"hello\" }");
    
    assert_token_type(&tokens[token_count - 1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_variable_assignment(void) {
    const char *source = "pg teamsQuery = `SELECT * FROM teams`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(5, token_count);
    
    assert_token_type(&tokens[0], TOKEN_IDENTIFIER);
    assert_token_value(&tokens[0], "pg");
    
    assert_token_type(&tokens[1], TOKEN_IDENTIFIER);
    assert_token_value(&tokens[1], "teamsQuery");
    
    assert_token_type(&tokens[2], TOKEN_EQUALS);
    
    assert_token_type(&tokens[3], TOKEN_STRING);
    assert_token_value(&tokens[3], "SELECT * FROM teams");
    
    assert_token_type(&tokens[4], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_result_step(void) {
    const char *source = "result\n  ok(200):\n    |> jq: `{ success: true }`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_GREATER_THAN(10, token_count);
    
    assert_token_type(&tokens[0], TOKEN_IDENTIFIER);
    assert_token_value(&tokens[0], "result");
    
    assert_token_type(&tokens[1], TOKEN_NEWLINE);
    
    assert_token_type(&tokens[2], TOKEN_IDENTIFIER);
    assert_token_value(&tokens[2], "ok");
    
    assert_token_type(&tokens[3], TOKEN_LPAREN);
    
    assert_token_type(&tokens[4], TOKEN_NUMBER);
    assert_token_value(&tokens[4], "200");
    
    assert_token_type(&tokens[5], TOKEN_RPAREN);
    
    assert_token_type(&tokens[6], TOKEN_COLON);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_empty_string(void) {
    const char *source = "";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(1, token_count);
    assert_token_type(&tokens[0], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_whitespace_only(void) {
    const char *source = "   \t\n  \n  ";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(3, token_count);
    assert_token_type(&tokens[0], TOKEN_NEWLINE);
    assert_token_type(&tokens[1], TOKEN_NEWLINE);
    assert_token_type(&tokens[2], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_line_column_tracking(void) {
    const char *source = "GET /test\n  |> jq: `hello`\n\nPOST /users";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    // Check line numbers for key tokens
    TEST_ASSERT_EQUAL(1, tokens[0].line);  // GET
    TEST_ASSERT_EQUAL(1, tokens[1].line);  // /test
    TEST_ASSERT_EQUAL(1, tokens[2].line);  // first newline
    TEST_ASSERT_EQUAL(2, tokens[3].line);  // |>
    TEST_ASSERT_EQUAL(2, tokens[4].line);  // jq
    TEST_ASSERT_EQUAL(2, tokens[5].line);  // :
    TEST_ASSERT_EQUAL(2, tokens[6].line);  // string
    TEST_ASSERT_EQUAL(2, tokens[7].line);  // second newline
    TEST_ASSERT_EQUAL(3, tokens[8].line);  // third newline
    
    // Find POST token
    for (int i = 0; i < token_count; i++) {
        if (tokens[i].type == TOKEN_HTTP_METHOD && 
            tokens[i].value && strcmp(tokens[i].value, "POST") == 0) {
            TEST_ASSERT_EQUAL(4, tokens[i].line);
            break;
        }
    }
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_unclosed_string(void) {
    const char *source = "`unclosed string";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    // Should handle unclosed string gracefully
    TEST_ASSERT_GREATER_THAN(0, token_count);
    assert_token_type(&tokens[token_count - 1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

static void test_lexer_tokenize_escaped_backticks(void) {
    const char *source = "`string with \\` escaped backtick`";
    int token_count;
    Token *tokens = tokenize_test_string(source, &token_count);
    
    TEST_ASSERT_EQUAL(2, token_count);
    assert_token_type(&tokens[0], TOKEN_STRING);
    // The lexer should handle escaped backticks appropriately
    assert_token_type(&tokens[1], TOKEN_EOF);
    
    free_test_tokens(tokens, token_count);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_lexer_new_and_free);
    RUN_TEST(test_lexer_peek_and_advance);
    RUN_TEST(test_lexer_skip_whitespace);
    RUN_TEST(test_lexer_skip_whitespace_with_newlines);
    RUN_TEST(test_lexer_tokenize_http_methods);
    RUN_TEST(test_lexer_tokenize_route_patterns);
    RUN_TEST(test_lexer_tokenize_pipeline_operator);
    RUN_TEST(test_lexer_tokenize_string_literals);
    RUN_TEST(test_lexer_tokenize_multiline_string);
    RUN_TEST(test_lexer_tokenize_identifiers);
    RUN_TEST(test_lexer_tokenize_numbers);
    RUN_TEST(test_lexer_tokenize_punctuation);
    RUN_TEST(test_lexer_tokenize_simple_route);
    RUN_TEST(test_lexer_tokenize_variable_assignment);
    RUN_TEST(test_lexer_tokenize_result_step);
    RUN_TEST(test_lexer_tokenize_empty_string);
    RUN_TEST(test_lexer_tokenize_whitespace_only);
    RUN_TEST(test_lexer_tokenize_line_column_tracking);
    RUN_TEST(test_lexer_tokenize_unclosed_string);
    RUN_TEST(test_lexer_tokenize_escaped_backticks);
    
    return UNITY_END();
}

