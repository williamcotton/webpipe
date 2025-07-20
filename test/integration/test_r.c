#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual r middleware
#include <dlfcn.h>
static void *r_middleware_handle = NULL;
static json_t *(*r_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;
static int (*r_middleware_init)(json_t *) = NULL;

static int load_r_middleware(void) {
    if (r_middleware_handle) return 0; // Already loaded
    
    r_middleware_handle = dlopen("./middleware/r.so", RTLD_LAZY);
    if (!r_middleware_handle) {
        fprintf(stderr, "Failed to load r middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(r_middleware_handle, "middleware_execute");
    r_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                        (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in r middleware: %s\n", dlerror());
        dlclose(r_middleware_handle);
        r_middleware_handle = NULL;
        return -1;
    }
    
    void *middleware_init_func = dlsym(r_middleware_handle, "middleware_init");
    r_middleware_init = (int (*)(json_t *))
                        (uintptr_t)middleware_init_func;
    if (!middleware_init_func) {
        fprintf(stderr, "Failed to find middleware_init in r middleware: %s\n", dlerror());
        dlclose(r_middleware_handle);
        r_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_r_middleware(void) {
    if (r_middleware_handle) {
        dlclose(r_middleware_handle);
        r_middleware_handle = NULL;
        r_middleware_execute = NULL;
        r_middleware_init = NULL;
    }
}

void setUp(void) {
    // Set up function called before each test
    if (load_r_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load r middleware");
    }
    
    // Initialize R middleware
    if (r_middleware_init(json_object()) != 0) {
        TEST_FAIL_MESSAGE("Failed to initialize r middleware");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    unload_r_middleware();
}

static void test_r_middleware_simple_hello(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/r/hello");
    const char *config = "list(message = \"Hello from R!\", timestamp = Sys.time(), r_version = R.version.string)";
    
    json_t *middleware_config = json_object();
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = r_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, middleware_config, &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *message = json_object_get(output, "message");
    json_t *timestamp = json_object_get(output, "timestamp");
    json_t *r_version = json_object_get(output, "r_version");
    
    TEST_ASSERT_NOT_NULL(message);
    TEST_ASSERT_NOT_NULL(timestamp);
    TEST_ASSERT_NOT_NULL(r_version);
    TEST_ASSERT_STRING_EQUAL("Hello from R!", json_string_value(message));
    
    json_decref(input);
    json_decref(output);
    json_decref(middleware_config);
    json_decref(variables);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_r_middleware_simple_hello);
    
    return UNITY_END();
}
