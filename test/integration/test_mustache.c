#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual mustache middleware
#include <dlfcn.h>
static void *mustache_middleware_handle = NULL;
static json_t *(*mustache_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *) = NULL;

static int load_mustache_middleware(void) {
    if (mustache_middleware_handle) return 0; // Already loaded
    
    mustache_middleware_handle = dlopen("./middleware/mustache.so", RTLD_LAZY);
    if (!mustache_middleware_handle) {
        fprintf(stderr, "Failed to load mustache middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(mustache_middleware_handle, "middleware_execute");
    mustache_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *))
                        (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in mustache middleware: %s\n", dlerror());
        dlclose(mustache_middleware_handle);
        mustache_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_mustache_middleware(void) {
    if (mustache_middleware_handle) {
        dlclose(mustache_middleware_handle);
        mustache_middleware_handle = NULL;
        mustache_middleware_execute = NULL;
    }
}

void setUp(void) {
    // Set up function called before each test
    if (load_mustache_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load mustache middleware");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    unload_mustache_middleware();
}

static void test_mustache_basic_template(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "name", json_string("World"));
    json_object_set_new(input, "message", json_string("Hello from test"));
    
    // Template
    const char *template = "<h1>{{message}}</h1><p>Hello, {{name}}!</p>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    
    // Verify content type
    TEST_ASSERT_NOT_NULL(content_type);
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "Hello from test") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "Hello, World!") != NULL);
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

static void test_mustache_template_with_loop(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON with array
    json_t *input = json_object();
    json_t *items = json_array();
    json_array_append_new(items, json_string("Item 1"));
    json_array_append_new(items, json_string("Item 2"));
    json_array_append_new(items, json_string("Item 3"));
    json_object_set_new(input, "items", items);
    
    // Template with loop
    const char *template = "<ul>{{#items}}<li>{{.}}</li>{{/items}}</ul>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "<ul>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<li>Item 1</li>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<li>Item 2</li>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<li>Item 3</li>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "</ul>") != NULL);
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

static void test_mustache_template_with_conditionals(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON with boolean
    json_t *input = json_object();
    json_object_set_new(input, "showMessage", json_true());
    json_object_set_new(input, "hideMessage", json_false());
    json_object_set_new(input, "message", json_string("This should show"));
    
    // Template with conditionals
    const char *template = "{{#showMessage}}<p>{{message}}</p>{{/showMessage}}{{#hideMessage}}<p>Hidden</p>{{/hideMessage}}";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "This should show") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "Hidden") == NULL);
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

static void test_mustache_template_error_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "name", json_string("Test"));
    
    // Invalid template (unmatched braces)
    const char *template = "{{#invalid}}<p>{{name}}</p>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result is an error object
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    // Verify content type is not set (should be NULL for errors)
    TEST_ASSERT_NULL(content_type);
    
    // Verify error structure
    json_t *errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL_INT(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    TEST_ASSERT_NOT_NULL(error);
    
    json_t *error_type = json_object_get(error, "type");
    TEST_ASSERT_NOT_NULL(error_type);
    TEST_ASSERT_EQUAL_STRING("templateError", json_string_value(error_type));
    
    json_t *error_message = json_object_get(error, "message");
    TEST_ASSERT_NOT_NULL(error_message);
    TEST_ASSERT_EQUAL_STRING("Template rendering failed", json_string_value(error_message));
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

static void test_mustache_empty_template(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "name", json_string("Test"));
    
    // Empty template
    const char *template = "";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content is empty
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_EQUAL_STRING("", html);
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

static void test_mustache_missing_variables(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON without expected variables
    json_t *input = json_object();
    json_object_set_new(input, "name", json_string("Test"));
    
    // Template with missing variable
    const char *template = "<h1>{{title}}</h1><p>{{name}}</p>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, &content_type, NULL);
    
    // Verify result (mustache should render with empty values for missing variables)
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "<h1></h1>") != NULL); // Empty title
    TEST_ASSERT_TRUE(strstr(html, "<p>Test</p>") != NULL); // Present name
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_mustache_basic_template);
    RUN_TEST(test_mustache_template_with_loop);
    RUN_TEST(test_mustache_template_with_conditionals);
    RUN_TEST(test_mustache_template_error_handling);
    RUN_TEST(test_mustache_empty_template);
    RUN_TEST(test_mustache_missing_variables);
    
    return UNITY_END();
}
