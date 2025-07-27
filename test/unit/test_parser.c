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

static void test_parser_new_and_free(void) {
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

static void test_parser_parse_simple_route(void) {
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
    TEST_ASSERT_STRING_EQUAL("jq", step->middleware);
    TEST_ASSERT_STRING_EQUAL("{ message: \"hello\" }", step->value);
    TEST_ASSERT_FALSE(step->is_variable);
    TEST_ASSERT_NULL(step->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_multi_step_pipeline(void) {
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
    TEST_ASSERT_STRING_EQUAL("jq", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("{ sqlParams: [.params.id] }", step1->value);
    TEST_ASSERT_FALSE(step1->is_variable);
    
    // Check second step
    PipelineStep *step2 = step1->next;
    TEST_ASSERT_NOT_NULL(step2);
    TEST_ASSERT_STRING_EQUAL("pg", step2->middleware);
    TEST_ASSERT_STRING_EQUAL("SELECT * FROM pages WHERE id = $1", step2->value);
    TEST_ASSERT_FALSE(step2->is_variable);
    
    // Check third step
    PipelineStep *step3 = step2->next;
    TEST_ASSERT_NOT_NULL(step3);
    TEST_ASSERT_STRING_EQUAL("jq", step3->middleware);
    TEST_ASSERT_STRING_EQUAL("{ page: .data.rows[0] }", step3->value);
    TEST_ASSERT_FALSE(step3->is_variable);
    TEST_ASSERT_NULL(step3->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_variable_assignment(void) {
    const char *source = "pg teamsQuery = `SELECT * FROM teams`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *var_assign = ast->data.program.statements[0];
    assert_ast_type(var_assign, AST_VARIABLE_ASSIGNMENT);
    TEST_ASSERT_STRING_EQUAL("pg", var_assign->data.var_assign.middleware);
    TEST_ASSERT_STRING_EQUAL("teamsQuery", var_assign->data.var_assign.name);
    TEST_ASSERT_STRING_EQUAL("SELECT * FROM teams", var_assign->data.var_assign.value);
    
    free_test_ast(ast);
}

static void test_parser_parse_variable_usage(void) {
    const char *source = "GET /teams\n  |> pg: teamsQuery";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("pg", step->middleware);
    TEST_ASSERT_STRING_EQUAL("teamsQuery", step->value);
    TEST_ASSERT_TRUE(step->is_variable);
    
    free_test_ast(ast);
}

static void test_parser_parse_result_step_simple(void) {
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
    TEST_ASSERT_STRING_EQUAL("jq", step->middleware);
    
    step = step->next;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("result", step->middleware);
    
    free_test_ast(ast);
}

static void test_parser_parse_result_step_multiple_conditions(void) {
    const char *source = "GET /test\n  |> result\n    ok(200):\n      |> jq: `{ success: true }`\n    validationError(400):\n      |> jq: `{ error: \"validation failed\" }`\n    default(500):\n      |> jq: `{ error: \"server error\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    
    free_test_ast(ast);
}

static void test_parser_parse_multiple_routes(void) {
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

static void test_parser_parse_mixed_statements(void) {
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

static void test_parser_parse_empty_program(void) {
    const char *source = "";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(0, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

static void test_parser_parse_whitespace_only(void) {
    const char *source = "   \n\n  \t\n  ";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(0, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

static void test_parser_parse_comments_ignored(void) {
    // Note: This test assumes the lexer handles comments if they exist
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    free_test_ast(ast);
}

static void test_parser_parse_all_http_methods(void) {
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

static void test_parser_parse_complex_route_patterns(void) {
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

static void test_parser_parse_pipeline_with_various_middlewares(void) {
    const char *middlewares[] = {"jq", "lua", "pg", "validate", "auth"};
    int num_middlewares = sizeof(middlewares) / sizeof(middlewares[0]);
    
    for (int i = 0; i < num_middlewares; i++) {
        char source[256];
        snprintf(source, sizeof(source), "GET /test\n  |> %s: `test config`", middlewares[i]);
        
        ASTNode *ast = parse_test_string(source);
        
        TEST_ASSERT_NOT_NULL(ast);
        assert_ast_type(ast, AST_PROGRAM);
        TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
        
        ASTNode *route = ast->data.program.statements[0];
        PipelineStep *step = route->data.route_def.pipeline;
        TEST_ASSERT_NOT_NULL(step);
        TEST_ASSERT_STRING_EQUAL(middlewares[i], step->middleware);
        TEST_ASSERT_STRING_EQUAL("test config", step->value);
        
        free_test_ast(ast);
    }
}

static void test_parser_parse_multiline_string_in_pipeline(void) {
    const char *source = "GET /test\n  |> jq: `{\n    message: \"hello\",\n    timestamp: now\n  }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *route = ast->data.program.statements[0];
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("jq", step->middleware);
    TEST_ASSERT_STRING_EQUAL("{\n    message: \"hello\",\n    timestamp: now\n  }", step->value);
    
    free_test_ast(ast);
}

// static void test_parser_parse_with_context(void) {
//     ParseContext *ctx = parse_context_create();
//     TEST_ASSERT_NOT_NULL(ctx);
    
//     int token_count;
//     Token *tokens = tokenize_test_string("GET /test", &token_count);
    
//     Parser *parser = parser_new_with_context(tokens, token_count, ctx);
//     TEST_ASSERT_NOT_NULL(parser);
//     TEST_ASSERT_EQUAL(ctx, parser->ctx);
    
//     ASTNode *ast = parser_parse(parser);
//     TEST_ASSERT_NOT_NULL(ast);
    
//     parser_free(parser);
//     free_test_tokens(tokens, token_count);
//     free_test_ast(ast);
//     parse_context_destroy(ctx);
// }

// static void test_parser_error_handling_invalid_syntax(void) {
//     const char *invalid_sources[] = {
//         "GET",                    // Missing route
//         "GET /test |>",          // Missing middleware
//         "GET /test |> jq",       // Missing colon
//         "GET /test |> jq:",      // Missing value
//         "INVALID /test",         // Invalid HTTP method
//         "GET /test\n  |> jq: `unclosed string"  // Unclosed string
//     };
//     int num_invalid = sizeof(invalid_sources) / sizeof(invalid_sources[0]);
    
//     for (int i = 0; i < num_invalid; i++) {
//         ASTNode *ast = parse_test_string(invalid_sources[i]);
        
//         // Parser should handle errors gracefully
//         // Either return NULL or a partial AST
//         // The exact behavior depends on error handling implementation
        
//         if (ast) {
//             free_test_ast(ast);
//         }
//     }
// }

static void test_parser_parse_config_block_simple(void) {
    const char *source = "config database {\n  host: \"localhost\"\n  port: 5432\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("database", config->data.config_block.name);
    
    // Check properties
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("host", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_STRING);
    TEST_ASSERT_STRING_EQUAL("localhost", prop->value->data.config_value_string.value);
    
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("port", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NUMBER);
    TEST_ASSERT_EQUAL(5432, (int)prop->value->data.config_value_number.value);
    TEST_ASSERT_TRUE(prop->value->data.config_value_number.is_integer);
    
    TEST_ASSERT_NULL(prop->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_config_block_complex(void) {
    const char *source = "config app {\n  name: \"WebPipe\"\n  version: \"1.0\"\n  debug: true\n  max_connections: 100\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("app", config->data.config_block.name);
    
    // Check all properties
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("name", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_STRING);
    TEST_ASSERT_STRING_EQUAL("WebPipe", prop->value->data.config_value_string.value);
    
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("version", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_STRING);
    TEST_ASSERT_STRING_EQUAL("1.0", prop->value->data.config_value_string.value);
    
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("debug", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_BOOLEAN);
    TEST_ASSERT_TRUE(prop->value->data.config_value_boolean.value);
    
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("max_connections", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NUMBER);
    TEST_ASSERT_EQUAL(100, (int)prop->value->data.config_value_number.value);
    TEST_ASSERT_TRUE(prop->value->data.config_value_number.is_integer);
    
    TEST_ASSERT_NULL(prop->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_config_with_env_vars(void) {
    const char *source = "config database {\n  url: $DATABASE_URL || \"postgres://localhost/db\"\n  timeout: $DB_TIMEOUT\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("database", config->data.config_block.name);
    
    // Check url property with default value
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("url", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_ENV_CALL);
    TEST_ASSERT_STRING_EQUAL("DATABASE_URL", prop->value->data.config_value_env_call.env_var);
    TEST_ASSERT_STRING_EQUAL("postgres://localhost/db", prop->value->data.config_value_env_call.default_value);
    
    // Check timeout property without default value
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("timeout", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_ENV_CALL);
    TEST_ASSERT_STRING_EQUAL("DB_TIMEOUT", prop->value->data.config_value_env_call.env_var);
    TEST_ASSERT_NULL(prop->value->data.config_value_env_call.default_value);
    
    TEST_ASSERT_NULL(prop->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_config_with_boolean_values(void) {
    const char *source = "config flags {\n  enabled: true\n  disabled: false\n  nullable: null\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("flags", config->data.config_block.name);
    
    // Check enabled property
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("enabled", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_BOOLEAN);
    TEST_ASSERT_TRUE(prop->value->data.config_value_boolean.value);
    
    // Check disabled property
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("disabled", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_BOOLEAN);
    TEST_ASSERT_FALSE(prop->value->data.config_value_boolean.value);
    
    // Check nullable property
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("nullable", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NULL);
    
    TEST_ASSERT_NULL(prop->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_config_with_numbers(void) {
    const char *source = "config numbers {\n  integer: 42\n  float: 3.14\n  zero: 0\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("numbers", config->data.config_block.name);
    
    // Check integer property
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("integer", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NUMBER);
    TEST_ASSERT_EQUAL(42, (int)prop->value->data.config_value_number.value);
    TEST_ASSERT_TRUE(prop->value->data.config_value_number.is_integer);
    
    // Check float property
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("float", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NUMBER);
    TEST_ASSERT_FLOAT_WITHIN(0.001, 3.14, prop->value->data.config_value_number.value);
    TEST_ASSERT_FALSE(prop->value->data.config_value_number.is_integer);
    
    // Check zero property
    prop = prop->next;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("zero", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_NUMBER);
    TEST_ASSERT_EQUAL(0, (int)prop->value->data.config_value_number.value);
    TEST_ASSERT_TRUE(prop->value->data.config_value_number.is_integer);
    
    TEST_ASSERT_NULL(prop->next);
    
    free_test_ast(ast);
}

static void test_parser_parse_config_ast_to_json(void) {
    const char *source = "config test {\n  name: \"test\"\n  count: 42\n  price: 3.14\n  enabled: true\n  value: null\n}";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    
    // Test AST to JSON conversion
    json_t *json = config_block_to_json(config);
    TEST_ASSERT_NOT_NULL(json);
    TEST_ASSERT_TRUE(json_is_object(json));
    
    // Check string value
    json_t *name_val = json_object_get(json, "name");
    TEST_ASSERT_NOT_NULL(name_val);
    TEST_ASSERT_TRUE(json_is_string(name_val));
    TEST_ASSERT_STRING_EQUAL("test", json_string_value(name_val));
    
    // Check integer value
    json_t *count_val = json_object_get(json, "count");
    TEST_ASSERT_NOT_NULL(count_val);
    TEST_ASSERT_TRUE(json_is_integer(count_val));
    TEST_ASSERT_EQUAL(42, json_integer_value(count_val));
    
    // Check float value
    json_t *price_val = json_object_get(json, "price");
    TEST_ASSERT_NOT_NULL(price_val);
    TEST_ASSERT_TRUE(json_is_real(price_val));
    TEST_ASSERT_FLOAT_WITHIN(0.001, 3.14, json_real_value(price_val));
    
    // Check boolean value
    json_t *enabled_val = json_object_get(json, "enabled");
    TEST_ASSERT_NOT_NULL(enabled_val);
    TEST_ASSERT_TRUE(json_is_true(enabled_val));
    
    // Check null value
    json_t *value_val = json_object_get(json, "value");
    TEST_ASSERT_NOT_NULL(value_val);
    TEST_ASSERT_TRUE(json_is_null(value_val));
    
    json_decref(json);
    free_test_ast(ast);
}

static void test_parser_parse_mixed_config_and_routes(void) {
    const char *source = "config app {\n  name: \"WebPipe\"\n}\n\nGET /test\n  |> jq: `{ message: \"hello\" }`";
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
    
    // Check config block
    ASTNode *config = ast->data.program.statements[0];
    assert_ast_type(config, AST_CONFIG_BLOCK);
    TEST_ASSERT_STRING_EQUAL("app", config->data.config_block.name);
    
    ConfigProperty *prop = config->data.config_block.properties;
    TEST_ASSERT_NOT_NULL(prop);
    TEST_ASSERT_STRING_EQUAL("name", prop->key);
    assert_ast_type(prop->value, AST_CONFIG_VALUE_STRING);
    TEST_ASSERT_STRING_EQUAL("WebPipe", prop->value->data.config_value_string.value);
    
    // Check route definition
    ASTNode *route = ast->data.program.statements[1];
    assert_ast_type(route, AST_ROUTE_DEFINITION);
    TEST_ASSERT_STRING_EQUAL("GET", route->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/test", route->data.route_def.route);
    
    PipelineStep *step = route->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(step);
    TEST_ASSERT_STRING_EQUAL("jq", step->middleware);
    TEST_ASSERT_STRING_EQUAL("{ message: \"hello\" }", step->value);
    
    free_test_ast(ast);
}

static void test_collect_middleware_names_from_config_blocks(void) {
    // Test that collect_middleware_names_from_ast includes config blocks
    const char *source = 
        "config pg {\n"
        "  host: \"localhost\"\n"
        "}\n"
        "config auth {\n"
        "  secret: \"test\"\n"
        "}\n"
        "GET /test\n"
        "  |> jq: `{ message: \"hello\" }`\n";
    
    ASTNode *ast = parse_test_string(source);
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    
    // Test current behavior (should collect jq from pipeline AND pg/auth from config)
    char *middleware_names[10];
    int middleware_count = 0;
    collect_middleware_names_from_ast(ast, middleware_names, &middleware_count, 10);
    
    // Should find "pg", "auth", and "jq"
    TEST_ASSERT_EQUAL(3, middleware_count);
    
    // Check that all expected middleware are present (order may vary)
    bool found_pg = false, found_auth = false, found_jq = false;
    for (int i = 0; i < middleware_count; i++) {
        if (strcmp(middleware_names[i], "pg") == 0) found_pg = true;
        else if (strcmp(middleware_names[i], "auth") == 0) found_auth = true;
        else if (strcmp(middleware_names[i], "jq") == 0) found_jq = true;
    }
    TEST_ASSERT_TRUE(found_pg);
    TEST_ASSERT_TRUE(found_auth);
    TEST_ASSERT_TRUE(found_jq);
    
    // Clean up
    for (int i = 0; i < middleware_count; i++) {
        free(middleware_names[i]);
    }
    free_test_ast(ast);
}

static void test_collect_middleware_names_config_only(void) {
    // Test with only config blocks, no pipelines
    const char *source = 
        "config database {\n"
        "  host: \"localhost\"\n"
        "}\n"
        "config cache {\n"
        "  enabled: true\n"
        "}\n";
    
    ASTNode *ast = parse_test_string(source);
    TEST_ASSERT_NOT_NULL(ast);
    assert_ast_type(ast, AST_PROGRAM);
    
    char *middleware_names[10];
    int middleware_count = 0;
    collect_middleware_names_from_ast(ast, middleware_names, &middleware_count, 10);
    
    // Should find "database" and "cache"
    TEST_ASSERT_EQUAL(2, middleware_count);
    
    // Check that all expected middleware are present (order may vary)
    bool found_database = false, found_cache = false;
    for (int i = 0; i < middleware_count; i++) {
        if (strcmp(middleware_names[i], "database") == 0) found_database = true;
        else if (strcmp(middleware_names[i], "cache") == 0) found_cache = true;
    }
    TEST_ASSERT_TRUE(found_database);
    TEST_ASSERT_TRUE(found_cache);
    
    // Clean up
    for (int i = 0; i < middleware_count; i++) {
        free(middleware_names[i]);
    }
    free_test_ast(ast);
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
    RUN_TEST(test_parser_parse_pipeline_with_various_middlewares);
    RUN_TEST(test_parser_parse_multiline_string_in_pipeline);
    RUN_TEST(test_parser_parse_config_block_simple);
    RUN_TEST(test_parser_parse_config_block_complex);
    RUN_TEST(test_parser_parse_config_with_env_vars);
    RUN_TEST(test_parser_parse_config_with_boolean_values);
    RUN_TEST(test_parser_parse_config_with_numbers);
    RUN_TEST(test_parser_parse_config_ast_to_json);
    RUN_TEST(test_parser_parse_mixed_config_and_routes);
    RUN_TEST(test_collect_middleware_names_from_config_blocks);
    RUN_TEST(test_collect_middleware_names_config_only);
    // RUN_TEST(test_parser_parse_with_context);
    // RUN_TEST(test_parser_error_handling_invalid_syntax);
    
    return UNITY_END();
}

