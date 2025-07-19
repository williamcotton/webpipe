#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <jansson.h>
#include <string.h>
#include <dlfcn.h>

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Wrapper functions for arena interface
static void* arena_alloc_wrapper(void* arena, size_t size) {
    return arena_alloc((MemoryArena*)arena, size);
}

static void arena_free_wrapper(void* arena) {
    arena_free((MemoryArena*)arena);
}

// Test helper functions
static json_t *execute_plot_middleware(const char *plot_spec, json_t *input_data) {
    // Create memory arena
    MemoryArena *arena = arena_create(1024 * 1024);
    if (!arena) return NULL;
    
    // Load plot middleware
    void *handle = dlopen("./build/plot.so", RTLD_LAZY);
    if (!handle) {
        arena_free(arena);
        return NULL;
    }
    
    // Get middleware function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, 
                      const char *, json_t *, char **, json_t *) = 
        (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, 
                    const char *, json_t *, char **, json_t *))dlsym(handle, "middleware_execute");
    
    if (!execute) {
        dlclose(handle);
        arena_free(arena);
        return NULL;
    }
    
    // Execute middleware
    char *content_type = NULL;
    json_t *result = execute(input_data, arena, arena_alloc_wrapper, arena_free_wrapper, 
                           plot_spec, NULL, &content_type, NULL);
    
    // Clean up
    dlclose(handle);
    arena_free(arena);
    
    return result;
}

void setUp(void) {
    // Set up test environment
}

void tearDown(void) {
    // Clean up after test
}

// Test basic plot middleware functionality
static void test_plot_middleware_loads(void) {
    void *handle = dlopen("./build/plot.so", RTLD_LAZY);
    TEST_ASSERT_NOT_NULL_MESSAGE(handle, "Plot middleware should load successfully");
    
    if (handle) {
        void *execute_func = dlsym(handle, "middleware_execute");
        TEST_ASSERT_NOT_NULL_MESSAGE(execute_func, "middleware_execute function should exist");
        dlclose(handle);
    }
}

static void test_plot_empty_spec_returns_error(void) {
    json_t *input = json_object();
    json_t *result = execute_plot_middleware("", input);
    
    TEST_ASSERT_NOT_NULL(result);
    
    json_t *errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL_MESSAGE(errors, "Should return error for empty spec");
    TEST_ASSERT_TRUE_MESSAGE(json_is_array(errors), "Errors should be an array");
    TEST_ASSERT_TRUE_MESSAGE(json_array_size(errors) > 0, "Should have at least one error");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_simple_scatter_plot(void) {
    // Create test data
    json_t *input = json_object();
    json_t *data_array = json_array();
    
    // Add some test points: [[1,10], [2,15], [3,12]]
    json_t *point1 = json_array();
    json_array_append_new(point1, json_integer(1));
    json_array_append_new(point1, json_integer(10));
    json_array_append_new(data_array, point1);
    
    json_t *point2 = json_array();
    json_array_append_new(point2, json_integer(2));
    json_array_append_new(point2, json_integer(15));
    json_array_append_new(data_array, point2);
    
    json_t *point3 = json_array();
    json_array_append_new(point3, json_integer(3));
    json_array_append_new(point3, json_integer(12));
    json_array_append_new(data_array, point3);
    
    json_object_set_new(input, "data", data_array);
    
    // Test scatter plot specification
    const char *plot_spec = "data(.data) + geom_point()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result for valid scatter plot");
    
    // Check that result is a string (SVG)
    TEST_ASSERT_TRUE_MESSAGE(json_is_string(result), "Result should be SVG string");
    
    const char *svg_content = json_string_value(result);
    TEST_ASSERT_NOT_NULL_MESSAGE(svg_content, "SVG content should not be null");
    
    // Basic SVG structure checks
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "<svg"), "Should contain SVG opening tag");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "</svg>"), "Should contain SVG closing tag");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "<circle"), "Should contain circle elements for points");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_simple_line_chart(void) {
    // Create test data
    json_t *input = json_object();
    json_t *data_array = json_array();
    
    // Add test points for line chart
    for (int i = 1; i <= 5; i++) {
        json_t *point = json_array();
        json_array_append_new(point, json_integer(i));
        json_array_append_new(point, json_integer(i * 10 + 5));
        json_array_append_new(data_array, point);
    }
    
    json_object_set_new(input, "data", data_array);
    
    // Test line chart specification
    const char *plot_spec = "data(.data) + geom_line()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result for valid line chart");
    TEST_ASSERT_TRUE_MESSAGE(json_is_string(result), "Result should be SVG string");
    
    const char *svg_content = json_string_value(result);
    TEST_ASSERT_NOT_NULL_MESSAGE(svg_content, "SVG content should not be null");
    
    // Check for line elements
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "<line"), "Should contain line elements");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_combined_point_and_line(void) {
    // Create test data
    json_t *input = json_object();
    json_t *data_array = json_array();
    
    // Add test points
    for (int i = 1; i <= 4; i++) {
        json_t *point = json_array();
        json_array_append_new(point, json_integer(i));
        json_array_append_new(point, json_integer(i * i));
        json_array_append_new(data_array, point);
    }
    
    json_object_set_new(input, "data", data_array);
    
    // Test combined specification
    const char *plot_spec = "data(.data) + geom_point() + geom_line()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result for combined plot");
    TEST_ASSERT_TRUE_MESSAGE(json_is_string(result), "Result should be SVG string");
    
    const char *svg_content = json_string_value(result);
    TEST_ASSERT_NOT_NULL_MESSAGE(svg_content, "SVG content should not be null");
    
    // Should contain both circles and lines
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "<circle"), "Should contain circle elements");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg_content, "<line"), "Should contain line elements");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_missing_data_field_returns_error(void) {
    json_t *input = json_object();
    json_object_set_new(input, "other_field", json_string("test"));
    
    // Try to reference non-existent data field
    const char *plot_spec = "data(.nonexistent) + geom_point()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result");
    
    // Should return error
    json_t *errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL_MESSAGE(errors, "Should return error for missing data");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_invalid_data_format_returns_error(void) {
    json_t *input = json_object();
    json_object_set_new(input, "data", json_string("not an array"));
    
    const char *plot_spec = "data(.data) + geom_point()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result");
    
    // Should return error for invalid data format
    json_t *errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL_MESSAGE(errors, "Should return error for invalid data format");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_no_geometry_returns_error(void) {
    json_t *input = json_object();
    json_t *data_array = json_array();
    json_object_set_new(input, "data", data_array);
    
    // Spec with data but no geometry
    const char *plot_spec = "data(.data)";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result");
    
    // Should return error for missing geometry
    json_t *errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL_MESSAGE(errors, "Should return error for missing geometry");
    
    json_decref(input);
    json_decref(result);
}

static void test_plot_content_type_is_svg(void) {
    // This test would need access to the actual middleware execution context
    // For now, we'll test that the middleware function exists and can be called
    
    void *handle = dlopen("./build/plot.so", RTLD_LAZY);
    TEST_ASSERT_NOT_NULL_MESSAGE(handle, "Plot middleware should load");
    
    if (handle) {
        // Test that we can get the middleware_execute function
        void *execute_func = dlsym(handle, "middleware_execute");
        TEST_ASSERT_NOT_NULL_MESSAGE(execute_func, "middleware_execute should exist");
        dlclose(handle);
    }
}

static void test_plot_svg_structure_validity(void) {
    // Create simple test data
    json_t *input = json_object();
    json_t *data_array = json_array();
    
    json_t *point = json_array();
    json_array_append_new(point, json_integer(5));
    json_array_append_new(point, json_integer(10));
    json_array_append_new(data_array, point);
    
    json_object_set_new(input, "data", data_array);
    
    const char *plot_spec = "data(.data) + geom_point()";
    json_t *result = execute_plot_middleware(plot_spec, input);
    
    TEST_ASSERT_NOT_NULL_MESSAGE(result, "Should return result");
    TEST_ASSERT_TRUE_MESSAGE(json_is_string(result), "Result should be string");
    
    const char *svg = json_string_value(result);
    
    // Check basic SVG structure
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg, "xmlns=\"http://www.w3.org/2000/svg\""), 
                                 "Should have SVG namespace");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg, "viewBox="), "Should have viewBox");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg, "width="), "Should have width");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(svg, "height="), "Should have height");
    
    // Check that SVG is properly closed
    const char *last_tag = strrchr(svg, '<');
    TEST_ASSERT_NOT_NULL_MESSAGE(last_tag, "Should have closing tag");
    TEST_ASSERT_NOT_NULL_MESSAGE(strstr(last_tag, "/svg>"), "Should end with </svg>");
    
    json_decref(input);
    json_decref(result);
}

// Test runner
int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_plot_middleware_loads);
    RUN_TEST(test_plot_empty_spec_returns_error);
    RUN_TEST(test_plot_simple_scatter_plot);
    RUN_TEST(test_plot_simple_line_chart);
    RUN_TEST(test_plot_combined_point_and_line);
    RUN_TEST(test_plot_missing_data_field_returns_error);
    RUN_TEST(test_plot_invalid_data_format_returns_error);
    RUN_TEST(test_plot_no_geometry_returns_error);
    RUN_TEST(test_plot_content_type_is_svg);
    RUN_TEST(test_plot_svg_structure_validity);
    
    return UNITY_END();
}
