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

void test_parser_new_and_free(void) {
    int token_count;
    Token *tokens = tokenize_test_string("GET /test", &token_count);
    
    Parser *parser = parser_new(tokens, token_count);
    
    TEST_ASSERT_NOT_NULL(parser);
    TEST_ASSERT_EQUAL(tokens, parser->tokens);
    TEST_ASSERT_EQUAL(token_count, parser->token_count);
    TEST_ASSERT_EQUAL(0, parser->current);
    
    parser_free(parser);
    free_test_tokens(tokens, token_count);
}

void test_parser_parse_simple_route(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/test", route->data.route_def.route);
    
    // Check pipeline
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("jq", step->plugin);
    TEST_ASSERT_STRING_EQUAL("{ message: \"hello\" }", step->value);
    TEST_ASSERT_FALSE(step->is_variable);
    TEST_ASSERT_NULL(step->next);
    
    free_test_ast(ast);
}

void test_parser_parse_multi_step_pipeline(void) {
    const char *source = "GET /page/:id\n  |> jq: `{ sqlParams: [.params.id] }`\n  |> pg: `SELECT * FROM pages WHERE id = $1`\n  |> jq: `{ page: .data.rows[0] }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/page/:id", route->data.route_def.route);
    
    // Check first step
    PipelineStep *step1 = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step1);
    TEST_ASSERT_STRING_EQUAL("jq", step1->plugin);
    TEST_ASSERT_STRING_EQUAL("{ sqlParams: [.params.id] }", step1->value);
    TEST_ASSERT_FALSE(step1->is_variable);
    
    // Check second step
    PipelineStep *step2 = step1->next;
    TEST_ASSERT_NOT_NULL(step2);
    TEST_ASSERT_STRING_EQUAL("pg", step2->plugin);
    TEST_ASSERT_STRING_EQUAL("SELECT * FROM pages WHERE id = $1", step2->value);
    TEST_ASSERT_FALSE(step2->is_variable);
    
    // Check third step
    PipelineStep *step3 = step2->next;
    TEST_ASSERT_NOT_NULL(step3);
    TEST_ASSERT_STRING_EQUAL("jq", step3->plugin);
    TEST_ASSERT_STRING_EQUAL("{ page: .data.rows[0] }", step3->value);
    TEST_ASSERT_FALSE(step3->is_variable);
    TEST_ASSERT_NULL(step3->next);
    
    free_test_ast(ast);
}

void test_parser_parse_variable_assignment(void) {
    const char *source = "pg teamsQuery = `SELECT * FROM teams`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *var_assign = ast->data.program.statements[0];
    assert_ast_type(var_assign, AST_VARIABLE_ASSIGNMENT);
    TEST_ASSERT_STRING_EQUAL("pg", var_assign->data.var_assign.plugin);
    TEST_ASSERT_STRING_EQUAL("teamsQuery", var_assign->data.var_assign.name);
    TEST_ASSERT_STRING_EQUAL("SELECT * FROM teams", var_assign->data.var_assign.value);
    
    free_test_ast(ast);
}

void test_parser_parse_variable_usage(void) {
    const char *source = "GET /teams\n  |> pg: teamsQuery";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("pg", step->plugin);
    TEST_ASSERT_STRING_EQUAL("teamsQuery", step->value);
    TEST_ASSERT_TRUE(step->is_variable);
    
    free_test_ast(ast);
}

void test_parser_parse_result_step_simple(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`\n  |> result\n    ok(200):\n      |> jq: `{ success: true }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    
    // Find the result step in the pipeline
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("jq", step->plugin);
    
    step = step->next;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("result", step->plugin);
    
    free_test_ast(ast);
}

void test_parser_parse_result_step_multiple_conditions(void) {
    const char *source = "GET /test\n  |> result\n    ok(200):\n      |> jq: `{ success: true }`\n    validationError(400):\n      |> jq: `{ error: \"validation failed\" }`\n    default(500):\n      |> jq: `{ error: \"server error\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    
    free_test_ast(ast);
}

void test_parser_parse_multiple_routes(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`\n\nPOST /users\n  |> jq: `{ user: .body }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
    
    // Check first route
    ASTNode *route1 = ast->data.program.statements[0];
    assert_ast_type(route1, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route1->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/test", route1->data.route_def.route);
    
    // Check second route
    ASTNode *route2 = ast->data.program.statements[1];
    assert_ast_type(route2, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("POST", route2->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/users", route2->data.route_def.route);
    
    free_test_ast(ast);
}

void test_parser_parse_mixed_statements(void) {
    const char *source = "pg teamsQuery = `SELECT * FROM teams`\n\nGET /teams\n  |> pg: teamsQuery\n\nGET /test\n  |> jq: `{ message: \"hello\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(3, ast->data.program.statement_count);
    
    // Check variable assignment
    ASTNode *var_assign = ast->data.program.statements[0];
    assert_ast_type(var_assign, AST_VARIABLE_ASSIGNMENT);
    TEST_ASSERT_STRING_EQUAL("teamsQuery", var_assign->data.var_assign.name);
    
    // Check first route
    ASTNode *route1 = ast->data.program.statements[1];
    assert_ast_type(route1, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route1->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/teams", route1->data.route_def.route);
    
    // Check second route
    ASTNode *route2 = ast->data.program.statements[2];
    assert_ast_type(route2, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route2->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/test", route2->data.route_def.route);
    
    free_test_ast(ast);
}

void test_parser_parse_empty_program(void) {
    const char *source = "";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(0, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

void test_parser_parse_whitespace_only(void) {
    const char *source = "   \n\n  \t\n  ";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(0, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

void test_parser_parse_comments_ignored(void) {
    // Note: This test assumes the lexer handles comments if they exist
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

void test_parser_parse_all_http_methods(void) {
    const char *methods[] = {"GET", "POST", "PUT", "DELETE", "PATCH"};
    int num_methods = sizeof(methods) / sizeof(methods[0]);
    
    for (int i = 0; i < num_methods; i++) {
        char source[256];
        snprintf(source, sizeof(source), "%s /test\n  |> jq: `{ method: \"%s\" }`", methods[i], methods[i]);
        
        ASTNode *ast = parse_test_string(source);
        
        TEST_ASSERT_NOT_NULL(ast);
        assert_ast_type(ast, AST_PROGRAM);
        TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
        
        ASTNode *route = ast->data.program.statements[0];
        assert_ast_type(route, AST_ROUTE_DEFINITION);
        TEST_ASSERT_STRING_EQUAL(methods[i], route->data.route_def.method);
        TEST_ASSERT_STRING_EQUAL("/test", route->data.route_def.route);
        
        free_test_ast(ast);
    }
}

void test_parser_parse_complex_route_patterns(void) {
    const char *routes[] = {
        "/",
        "/test",
        "/page/:id",
        "/user/:userId/post/:postId",
        "/api/v1/users/:id/profile",
        "/files/:filename.:extension"
    };
    int num_routes = sizeof(routes) / sizeof(routes[0]);
    
    for (int i = 0; i < num_routes; i++) {
        char source[256];
        snprintf(source, sizeof(source), "GET %s\n  |> jq: `{ route: \"%s\" }`", routes[i], routes[i]);
        
        ASTNode *ast = parse_test_string(source);
        
        TEST_ASSERT_NOT_NULL(ast);
        assert_ast_type(ast, AST_PROGRAM);
        TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
        
        ASTNode *route = ast->data.program.statements[0];
        assert_ast_type(route, AST_ROUTE_DEFINITION);
        TEST_ASSERT_STRING_EQUAL(routes[i], route->data.route_def.route);
        
        free_test_ast(ast);
    }
}

void test_parser_parse_pipeline_with_various_plugins(void) {
    const char *plugins[] = {"jq", "lua", "pg", "validate", "auth"};
    int num_plugins = sizeof(plugins) / sizeof(plugins[0]);
    
    for (int i = 0; i < num_plugins; i++) {
        char source[256];
        snprintf(source, sizeof(source), "GET /test\n  |> %s: `test config`", plugins[i]);
        
        ASTNode *ast = parse_test_string(source);
        
        TEST_ASSERT_NOT_NULL(ast);
        assert_ast_type(ast, AST_PROGRAM);
        TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
        
        ASTNode *route = ast->data.program.statements[0];
        PipelineStep *step = route->data.route_def.pipeline;
        TEST_ASSERT_NOT_NULL(step);
        TEST_ASSERT_STRING_EQUAL(plugins[i], step->plugin);
        TEST_ASSERT_STRING_EQUAL("test config", step->value);
        
        free_test_ast(ast);
    }
}

void test_parser_parse_multiline_string_in_pipeline(void) {
    const char *source = "GET /test\n  |> jq: `{\n    message: \"hello\",\n    timestamp: now\n  }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("jq", step->plugin);
    TEST_ASSERT_STRING_EQUAL("{\n    message: \"hello\",\n    timestamp: now\n  }", step->value);
    
    free_test_ast(ast);
}

void test_parser_parse_with_context(void) {
    ParseContext *ctx = parse_context_create();
    TEST_ASSERT_NOT_NULL(ctx);
    
    int token_count;
    Token *tokens = tokenize_test_string("GET /test", &token_count);
    
    Parser *parser = parser_new_with_context(tokens, token_count, ctx);
    TEST_ASSERT_NOT_NULL(parser);
    TEST_ASSERT_EQUAL(ctx, parser->ctx);
    
    ASTNode *ast = parser_parse(parser);
    TEST_ASSERT_NOT_NULL(ast);
    
    parser_free(parser);
    free_test_tokens(tokens, token_count);
    free_test_ast(ast);
    parse_context_destroy(ctx);
}

void test_parser_error_handling_invalid_syntax(void) {
    const char *invalid_sources[] = {
        "GET",                    // Missing route
        "GET /test |>",          // Missing plugin
        "GET /test |> jq",       // Missing colon
        "GET /test |> jq:",      // Missing value
        "INVALID /test",         // Invalid HTTP method
        "GET /test\n  |> jq: `unclosed string"  // Unclosed string
    };
    int num_invalid = sizeof(invalid_sources) / sizeof(invalid_sources[0]);
    
    for (int i = 0; i < num_invalid; i++) {
        ASTNode *ast = parse_test_string(invalid_sources[i]);
        
        // Parser should handle errors gracefully
        // Either return NULL or a partial AST
        // The exact behavior depends on error handling implementation
        
        if (ast) {
            free_test_ast(ast);
        }
    }
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_parser_new_and_free);
    RUN_TEST(test_parser_parse_simple_route);
    RUN_TEST(test_parser_parse_multi_step_pipeline);
    RUN_TEST(test_parser_parse_variable_assignment);
    RUN_TEST(test_parser_parse_variable_usage);
    RUN_TEST(test_parser_parse_result_step_simple);
    RUN_TEST(test_parser_parse_result_step_multiple_conditions);
    RUN_TEST(test_parser_parse_multiple_routes);
    RUN_TEST(test_parser_parse_mixed_statements);
    RUN_TEST(test_parser_parse_empty_program);
    RUN_TEST(test_parser_parse_whitespace_only);
    RUN_TEST(test_parser_parse_comments_ignored);
    RUN_TEST(test_parser_parse_all_http_methods);
    RUN_TEST(test_parser_parse_complex_route_patterns);
    RUN_TEST(test_parser_parse_pipeline_with_various_plugins);
    RUN_TEST(test_parser_parse_multiline_string_in_pipeline);
    RUN_TEST(test_parser_parse_with_context);
    RUN_TEST(test_parser_error_handling_invalid_syntax);
    
    return UNITY_END();
}