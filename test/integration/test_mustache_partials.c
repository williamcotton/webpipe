#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual mustache middleware
#include <dlfcn.h>
static void *mustache_middleware_handle = NULL;
static json_t *(*mustache_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;

static int load_mustache_middleware(void) {
    if (mustache_middleware_handle) return 0; // Already loaded
    
    mustache_middleware_handle = dlopen("./middleware/mustache.so", RTLD_LAZY);
    if (!mustache_middleware_handle) {
        fprintf(stderr, "Failed to load mustache middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(mustache_middleware_handle, "middleware_execute");
    mustache_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
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

static void test_mustache_basic_partial(void) {
    MemoryArena *arena = create_test_arena(2048);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create variables with a partial
    json_t *variables = json_object();
    json_object_set_new(variables, "cardPartial", json_string("<div class=\"card\"><h3>{{title}}</h3></div>"));
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "title", json_string("Test Title"));
    
    // Template that uses the partial
    const char *template = "<html><body>{{>cardPartial}}</body></html>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, NULL, &content_type, variables);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    
    // Verify content type
    TEST_ASSERT_NOT_NULL(content_type);
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content includes rendered partial
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "<html><body>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<div class=\"card\">") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<h3>Test Title</h3>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "</div>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "</body></html>") != NULL);
    
    json_decref(input);
    json_decref(result);
    json_decref(variables);
    destroy_test_arena(arena);
}

static void test_mustache_nested_partials(void) {
    MemoryArena *arena = create_test_arena(2048);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create variables with nested partials
    json_t *variables = json_object();
    json_object_set_new(variables, "headerPartial", json_string("<header><h1>{{siteName}}</h1>{{>navPartial}}</header>"));
    json_object_set_new(variables, "navPartial", json_string("<nav><a href=\"/\">Home</a></nav>"));
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "siteName", json_string("My Site"));
    
    // Template that uses nested partials
    const char *template = "<html>{{>headerPartial}}<main>Content</main></html>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, NULL, &content_type, variables);
    
    // Verify result
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content includes nested partials
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "<header>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<h1>My Site</h1>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<nav>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "<a href=\"/\">Home</a>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "</nav>") != NULL);
    TEST_ASSERT_TRUE(strstr(html, "</header>") != NULL);
    
    json_decref(input);
    json_decref(result);
    json_decref(variables);
    destroy_test_arena(arena);
}

static void test_mustache_missing_partial(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create empty variables (no partials)
    json_t *variables = json_object();
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "title", json_string("Test"));
    
    // Template that references non-existent partial
    const char *template = "<html>{{>nonexistentPartial}}</html>";
    
    // Execute mustache middleware
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, NULL, &content_type, variables);
    
    // Verify result - should be an error or render with empty partial
    TEST_ASSERT_NOT_NULL(result);
    
    // Check if it's an error response or rendered with missing partial
    if (json_is_object(result)) {
        // Error response
        json_t *errors = json_object_get(result, "errors");
        TEST_ASSERT_NOT_NULL(errors);
        TEST_ASSERT_TRUE(json_is_array(errors));
        // Content type should be NULL for errors
        TEST_ASSERT_NULL(content_type);
    } else {
        // Rendered with missing partial (mustache might render empty)
        TEST_ASSERT_TRUE(json_is_string(result));
        TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    }
    
    json_decref(input);
    json_decref(result);
    json_decref(variables);
    destroy_test_arena(arena);
}

static void test_mustache_no_variables(void) {
    MemoryArena *arena = create_test_arena(1024);
    TEST_ASSERT_NOT_NULL(arena);
    
    // Create input JSON
    json_t *input = json_object();
    json_object_set_new(input, "title", json_string("Test"));
    
    // Template without partials
    const char *template = "<html><h1>{{title}}</h1></html>";
    
    // Execute mustache middleware with NULL variables
    char *content_type = NULL;
    json_t *result = mustache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, template, NULL, &content_type, NULL);
    
    // Verify result - should work fine without variables
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_string(result));
    TEST_ASSERT_EQUAL_STRING("text/html", content_type);
    
    // Verify HTML content
    const char *html = json_string_value(result);
    TEST_ASSERT_NOT_NULL(html);
    TEST_ASSERT_TRUE(strstr(html, "<h1>Test</h1>") != NULL);
    
    json_decref(input);
    json_decref(result);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_mustache_basic_partial);
    RUN_TEST(test_mustache_nested_partials);
    RUN_TEST(test_mustache_missing_partial);
    RUN_TEST(test_mustache_no_variables);
    
    return UNITY_END();
}
