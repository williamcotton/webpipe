#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

void setUp(void) {
    // Set up function called before each test
    setup_test_database();
}

void tearDown(void) {
    // Tear down function called after each test
    teardown_test_database();
}

void test_e2e_simple_route(void) {
    // Test the /test route from test.wp
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/test", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_parameterized_route(void) {
    // Test the /page/:id route from test.wp
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/page/123", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_t *team = json_object_get(json_response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_result_step_success(void) {
    // Test the /test3 route with result step
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/test3", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_t *success = json_object_get(json_response, "success");
    TEST_ASSERT_NOT_NULL(success);
    TEST_ASSERT_TRUE(json_is_true(success));
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_result_step_validation_error(void) {
    // Test the /test4 route with validation error
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/test4", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(400, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_t *error = json_object_get(json_response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Validation failed", json_string_value(error));
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_result_step_sql_error(void) {
    // Test the /test-sql-error route
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/test-sql-error", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(500, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_t *error = json_object_get(json_response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Database error", json_string_value(error));
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_variable_usage(void) {
    // Test the /teams route that uses teamsQuery variable
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/teams", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Parse response body
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    json_t *data = json_object_get(json_response, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *rows = json_object_get(data, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_pipeline_chain(void) {
    // Test a multi-step pipeline
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/page/123", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(response->body);
    
    // Verify the pipeline chain worked correctly
    json_t *json_response = parse_json_string(response->body);
    TEST_ASSERT_NOT_NULL(json_response);
    
    // Should have processed through jq -> pg -> jq
    json_t *team = json_object_get(json_response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(json_response);
    destroy_test_response(response);
}

void test_e2e_invalid_route(void) {
    // Test non-existent route
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("GET", "/nonexistent", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(404, response->status_code);
    
    destroy_test_response(response);
}

void test_e2e_invalid_method(void) {
    // Test invalid HTTP method
    struct test_http_response *response = create_test_response();
    
    int result = simulate_http_request("INVALID", "/test", NULL, response);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(405, response->status_code);
    
    destroy_test_response(response);
}

void test_e2e_concurrent_requests(void) {
    // Test concurrent request handling
    struct test_http_response *responses[5];
    
    // Create multiple responses
    for (int i = 0; i < 5; i++) {
        responses[i] = create_test_response();
    }
    
    // Simulate concurrent requests
    for (int i = 0; i < 5; i++) {
        int result = simulate_http_request("GET", "/test", NULL, responses[i]);
        TEST_ASSERT_EQUAL(0, result);
        TEST_ASSERT_EQUAL(200, responses[i]->status_code);
    }
    
    // Clean up
    for (int i = 0; i < 5; i++) {
        destroy_test_response(responses[i]);
    }
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_e2e_simple_route);
    RUN_TEST(test_e2e_parameterized_route);
    RUN_TEST(test_e2e_result_step_success);
    RUN_TEST(test_e2e_result_step_validation_error);
    RUN_TEST(test_e2e_result_step_sql_error);
    RUN_TEST(test_e2e_variable_usage);
    RUN_TEST(test_e2e_pipeline_chain);
    RUN_TEST(test_e2e_invalid_route);
    RUN_TEST(test_e2e_invalid_method);
    RUN_TEST(test_e2e_concurrent_requests);
    
    return UNITY_END();
}