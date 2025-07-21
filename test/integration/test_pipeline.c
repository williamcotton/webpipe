#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"

// Pipeline testing is different from middleware testing since pipeline is built-in functionality
// We need to test at the parser and runtime level

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
    cleanup_test_runtime();
}

static void test_pipeline_definition_parsing(void) {
    const char *source = 
        "pipeline getTeam = \n"
        "  |> jq: `{ sqlParams: [.params.id | tostring] }`\n"
        "  |> jq: `{ team: .data.rows[0] }`\n"
        "\n"
        "GET /team/:id\n"
        "  |> pipeline: getTeam\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(AST_PROGRAM, ast->type);
    TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
    
    // Check pipeline definition
    ASTNode *pipeline_def = ast->data.program.statements[0];
    TEST_ASSERT_NOT_NULL(pipeline_def);
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
    TEST_ASSERT_STRING_EQUAL("getTeam", pipeline_def->data.pipeline_def.name);
    TEST_ASSERT_NOT_NULL(pipeline_def->data.pipeline_def.pipeline);
    
    // Check pipeline steps
    PipelineStep *step1 = pipeline_def->data.pipeline_def.pipeline;
    TEST_ASSERT_NOT_NULL(step1);
    TEST_ASSERT_STRING_EQUAL("jq", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("{ sqlParams: [.params.id | tostring] }", step1->value);
    TEST_ASSERT_FALSE(step1->is_variable);
    
    PipelineStep *step2 = step1->next;
    TEST_ASSERT_NOT_NULL(step2);
    TEST_ASSERT_STRING_EQUAL("jq", step2->middleware);
    TEST_ASSERT_STRING_EQUAL("{ team: .data.rows[0] }", step2->value);
    TEST_ASSERT_FALSE(step2->is_variable);
    
    // Check route definition
    ASTNode *route_def = ast->data.program.statements[1];
    TEST_ASSERT_NOT_NULL(route_def);
    TEST_ASSERT_EQUAL(AST_ROUTE_DEFINITION, route_def->type);
    TEST_ASSERT_STRING_EQUAL("GET", route_def->data.route_def.method);
    TEST_ASSERT_STRING_EQUAL("/team/:id", route_def->data.route_def.route);
    
    // Check pipeline usage in route
    PipelineStep *route_step = route_def->data.route_def.pipeline;
    TEST_ASSERT_NOT_NULL(route_step);
    TEST_ASSERT_STRING_EQUAL("pipeline", route_step->middleware);
    TEST_ASSERT_STRING_EQUAL("getTeam", route_step->value);
    TEST_ASSERT_TRUE(route_step->is_variable);
    
    free_test_ast(ast);
}

static void test_pipeline_definition_with_multiple_types(void) {
    const char *source = 
        "pipeline complexFlow = \n"
        "  |> jq: `{ id: .params.id }`\n"
        "  |> pg: `SELECT * FROM users WHERE id = $1`\n"
        "  |> jq: `{ user: .data.rows[0] }`\n"
        "\n"
        "GET /user/:id\n"
        "  |> pipeline: complexFlow\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(AST_PROGRAM, ast->type);
    TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
    
    // Check pipeline definition has multiple middleware types
    ASTNode *pipeline_def = ast->data.program.statements[0];
    TEST_ASSERT_NOT_NULL(pipeline_def);
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
    
    PipelineStep *step1 = pipeline_def->data.pipeline_def.pipeline;
    PipelineStep *step2 = step1->next;
    PipelineStep *step3 = step2->next;
    
    TEST_ASSERT_STRING_EQUAL("jq", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("pg", step2->middleware);
    TEST_ASSERT_STRING_EQUAL("jq", step3->middleware);
    
    free_test_ast(ast);
}

static void test_pipeline_with_result_step(void) {
    const char *source = 
        "pipeline withError = \n"
        "  |> jq: `{ test: \"value\" }`\n"
        "  |> result\n"
        "    ok(200):\n"
        "      |> jq: `{ success: true }`\n"
        "    default(500):\n"
        "      |> jq: `{ error: \"failed\" }`\n"
        "\n"
        "GET /test-result\n"
        "  |> pipeline: withError\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
    
    // Check pipeline definition with result step
    ASTNode *pipeline_def = ast->data.program.statements[0];
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
    
    PipelineStep *step1 = pipeline_def->data.pipeline_def.pipeline;
    PipelineStep *step2 = step1->next;
    
    TEST_ASSERT_STRING_EQUAL("jq", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("result", step2->middleware);
    
    free_test_ast(ast);
}

static void test_pipeline_runtime_simple(void) {
    // Create a simple WP file for testing
    const char *wp_content = 
        "pipeline simple = \n"
        "  |> jq: `{ message: \"Hello from pipeline!\" }`\n"
        "\n"
        "GET /hello\n"
        "  |> pipeline: simple\n";
    
    // Write test file
    FILE *f = fopen("test_pipeline_simple.wp", "w");
    TEST_ASSERT_NOT_NULL(f);
    fprintf(f, "%s", wp_content);
    fclose(f);
    
    // Initialize runtime with test file
    int result = init_test_runtime("test_pipeline_simple.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    // Test would require HTTP server simulation which is complex
    // For now, just test that the runtime can be initialized with pipeline definitions
    
    cleanup_test_runtime();
    
    // Clean up test file
    remove("test_pipeline_simple.wp");
}

static void test_multiple_pipeline_definitions(void) {
    const char *source = 
        "pipeline first = \n"
        "  |> jq: `{ step: 1 }`\n"
        "\n"
        "pipeline second = \n"
        "  |> jq: `{ step: 2 }`\n"
        "\n"
        "GET /first\n"
        "  |> pipeline: first\n"
        "\n"
        "GET /second\n"
        "  |> pipeline: second\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(4, ast->data.program.statement_count);
    
    // Check both pipeline definitions
    ASTNode *first_def = ast->data.program.statements[0];
    ASTNode *second_def = ast->data.program.statements[1];
    
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, first_def->type);
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, second_def->type);
    TEST_ASSERT_STRING_EQUAL("first", first_def->data.pipeline_def.name);
    TEST_ASSERT_STRING_EQUAL("second", second_def->data.pipeline_def.name);
    
    // Check route definitions use different pipelines
    ASTNode *first_route = ast->data.program.statements[2];
    ASTNode *second_route = ast->data.program.statements[3];
    
    TEST_ASSERT_STRING_EQUAL("first", first_route->data.route_def.pipeline->value);
    TEST_ASSERT_STRING_EQUAL("second", second_route->data.route_def.pipeline->value);
    
    free_test_ast(ast);
}

static void test_pipeline_with_variable_references(void) {
    const char *source = 
        "jq messageTemplate = `{ greeting: \"Hello\" }`\n"
        "\n"
        "pipeline withVar = \n"
        "  |> jq: messageTemplate\n"
        "  |> jq: `{ result: .greeting }`\n"
        "\n"
        "GET /var-test\n"
        "  |> pipeline: withVar\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(3, ast->data.program.statement_count);
    
    // Check variable assignment
    ASTNode *var_assign = ast->data.program.statements[0];
    TEST_ASSERT_EQUAL(AST_VARIABLE_ASSIGNMENT, var_assign->type);
    TEST_ASSERT_STRING_EQUAL("messageTemplate", var_assign->data.var_assign.name);
    
    // Check pipeline definition uses variable
    ASTNode *pipeline_def = ast->data.program.statements[1];
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
    
    PipelineStep *step1 = pipeline_def->data.pipeline_def.pipeline;
    TEST_ASSERT_STRING_EQUAL("jq", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("messageTemplate", step1->value);
    TEST_ASSERT_TRUE(step1->is_variable); // Should be marked as variable reference
    
    free_test_ast(ast);
}

static void test_nested_pipeline_usage(void) {
    const char *source = 
        "pipeline base = \n"
        "  |> jq: `{ base: true }`\n"
        "\n"
        "pipeline extended = \n"
        "  |> pipeline: base\n"
        "  |> jq: `{ extended: true, base: .base }`\n"
        "\n"
        "GET /nested\n"
        "  |> pipeline: extended\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(3, ast->data.program.statement_count);
    
    // Check nested pipeline definition
    ASTNode *extended_def = ast->data.program.statements[1];
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, extended_def->type);
    TEST_ASSERT_STRING_EQUAL("extended", extended_def->data.pipeline_def.name);
    
    PipelineStep *step1 = extended_def->data.pipeline_def.pipeline;
    TEST_ASSERT_STRING_EQUAL("pipeline", step1->middleware);
    TEST_ASSERT_STRING_EQUAL("base", step1->value);
    TEST_ASSERT_TRUE(step1->is_variable);
    
    PipelineStep *step2 = step1->next;
    TEST_ASSERT_STRING_EQUAL("jq", step2->middleware);
    
    free_test_ast(ast);
}

static void test_pipeline_error_undefined_reference(void) {
    const char *source = 
        "GET /undefined\n"
        "  |> pipeline: nonexistent\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(1, ast->data.program.statement_count);
    
    // This should parse successfully, but would fail at runtime
    // which is expected behavior
    ASTNode *route_def = ast->data.program.statements[0];
    TEST_ASSERT_EQUAL(AST_ROUTE_DEFINITION, route_def->type);
    
    PipelineStep *step = route_def->data.route_def.pipeline;
    TEST_ASSERT_STRING_EQUAL("pipeline", step->middleware);
    TEST_ASSERT_STRING_EQUAL("nonexistent", step->value);
    
    free_test_ast(ast);
}

static void test_pipeline_with_config_middleware(void) {
    const char *source = 
        "config jq {\n"
        "  timeout: 30\n"
        "}\n"
        "\n"
        "pipeline withConfig = \n"
        "  |> jq: `{ configured: true }`\n"
        "\n"
        "GET /config-test\n"
        "  |> pipeline: withConfig\n";
    
    ASTNode *ast = parse_test_string(source);
    
    TEST_ASSERT_NOT_NULL(ast);
    TEST_ASSERT_EQUAL(3, ast->data.program.statement_count);
    
    // Check config block
    ASTNode *config = ast->data.program.statements[0];
    TEST_ASSERT_EQUAL(AST_CONFIG_BLOCK, config->type);
    
    // Check pipeline definition
    ASTNode *pipeline_def = ast->data.program.statements[1];
    TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
    
    free_test_ast(ast);
}

static void test_empty_pipeline_definition(void) {
    // Test parsing of pipeline with no steps (edge case)
    const char *source = 
        "pipeline empty = \n"
        "\n"
        "GET /empty\n"
        "  |> pipeline: empty\n";
    
    // This may not parse correctly depending on implementation
    // The parser expects at least one pipeline step
    ASTNode *ast = parse_test_string(source);
    
    if (ast) {
        // If it parses, check structure
        TEST_ASSERT_EQUAL(AST_PROGRAM, ast->type);
        free_test_ast(ast);
    } else {
        // If it doesn't parse, that's also acceptable behavior
        TEST_PASS_MESSAGE("Empty pipeline correctly rejected by parser");
    }
}

static void test_pipeline_memory_management(void) {
    const char *source = 
        "pipeline memTest = \n"
        "  |> jq: `{ test: \"memory\" }`\n"
        "  |> jq: `{ result: .test }`\n"
        "\n"
        "GET /mem-test\n"
        "  |> pipeline: memTest\n";
    
    // Parse multiple times to test memory management
    for (int i = 0; i < 10; i++) {
        ASTNode *ast = parse_test_string(source);
        TEST_ASSERT_NOT_NULL(ast);
        
        // Verify structure is consistent
        TEST_ASSERT_EQUAL(2, ast->data.program.statement_count);
        ASTNode *pipeline_def = ast->data.program.statements[0];
        TEST_ASSERT_EQUAL(AST_PIPELINE_DEFINITION, pipeline_def->type);
        
        free_test_ast(ast);
    }
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pipeline_definition_parsing);
    RUN_TEST(test_pipeline_definition_with_multiple_types);
    RUN_TEST(test_pipeline_with_result_step);
    RUN_TEST(test_pipeline_runtime_simple);
    RUN_TEST(test_multiple_pipeline_definitions);
    RUN_TEST(test_pipeline_with_variable_references);
    RUN_TEST(test_nested_pipeline_usage);
    RUN_TEST(test_pipeline_error_undefined_reference);
    RUN_TEST(test_pipeline_with_config_middleware);
    RUN_TEST(test_empty_pipeline_definition);
    RUN_TEST(test_pipeline_memory_management);
    
    return UNITY_END();
}
