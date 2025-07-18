#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual auth middleware
#include <dlfcn.h>
static void *auth_middleware_handle = NULL;
static json_t *(*auth_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;

static int load_auth_middleware(void) {
    if (auth_middleware_handle) return 0; // Already loaded
    
    auth_middleware_handle = dlopen("./middleware/auth.so", RTLD_LAZY);
    if (!auth_middleware_handle) {
        fprintf(stderr, "Failed to load auth middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(auth_middleware_handle, "middleware_execute");
    auth_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                         (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in auth middleware: %s\n", dlerror());
        dlclose(auth_middleware_handle);
        auth_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_auth_middleware(void) {
    if (auth_middleware_handle) {
        dlclose(auth_middleware_handle);
        auth_middleware_handle = NULL;
        auth_middleware_execute = NULL;
    }
}

void setUp(void) {
    // Set up function called before each test
    if (load_auth_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load auth middleware");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    unload_auth_middleware();
}

static void test_auth_middleware_login_missing_body(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("POST", "/auth/login");
    const char *config = "login";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Missing login or password", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_login_missing_credentials(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "username", json_string("testuser"));
    // Missing password
    json_t *input = create_test_request_with_body("POST", "/auth/login", body);
    
    const char *config = "login";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Missing login or password", json_string_value(message));
    }
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_register_missing_body(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("POST", "/auth/register");
    const char *config = "register";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Missing required fields: login, email, password", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_register_missing_fields(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "login", json_string("testuser"));
    json_object_set_new(body, "email", json_string("test@example.com"));
    // Missing password
    json_t *input = create_test_request_with_body("POST", "/auth/register", body);
    
    const char *config = "register";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Missing required fields: login, email, password", json_string_value(message));
    }
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_required_auth_no_cookies(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/protected");
    const char *config = "required";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Authentication required", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_required_auth_no_session_cookie(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *cookies = json_object();
    json_object_set_new(cookies, "other_cookie", json_string("value"));
    json_t *input = create_test_request_with_cookies("GET", "/protected", cookies);
    
    const char *config = "required";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Authentication required", json_string_value(message));
    }
    
    json_decref(cookies);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_optional_auth_no_cookies(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/maybe-protected");
    const char *config = "optional";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should return input as-is (no errors)
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    // Should not have user object
    json_t *user = json_object_get(output, "user");
    TEST_ASSERT_NULL(user);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_optional_auth_no_session_cookie(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *cookies = json_object();
    json_object_set_new(cookies, "other_cookie", json_string("value"));
    json_t *input = create_test_request_with_cookies("GET", "/maybe-protected", cookies);
    
    const char *config = "optional";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should return input as-is (no errors)
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    // Should not have user object
    json_t *user = json_object_get(output, "user");
    TEST_ASSERT_NULL(user);
    
    json_decref(cookies);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_logout_no_cookies(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("POST", "/auth/logout");
    const char *config = "logout";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("No cookies found", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_logout_no_session_cookie(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *cookies = json_object();
    json_object_set_new(cookies, "other_cookie", json_string("value"));
    json_t *input = create_test_request_with_cookies("POST", "/auth/logout", cookies);
    
    const char *config = "logout";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("No session cookie found", json_string_value(message));
    }
    
    json_decref(cookies);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_type_check_admin(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/admin");
    const char *config = "type:admin";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should require authentication first
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Authentication required", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_invalid_config(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "invalid_operation";
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("Invalid auth configuration", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_auth_middleware_null_config(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    json_t *output = auth_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, NULL, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error structure
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
        
        json_t *error = json_array_get(errors, 0);
        json_t *message = json_object_get(error, "message");
        TEST_ASSERT_NOT_NULL(message);
        TEST_ASSERT_STRING_EQUAL("No auth configuration provided", json_string_value(message));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_auth_middleware_login_missing_body);
    RUN_TEST(test_auth_middleware_login_missing_credentials);
    RUN_TEST(test_auth_middleware_register_missing_body);
    RUN_TEST(test_auth_middleware_register_missing_fields);
    RUN_TEST(test_auth_middleware_required_auth_no_cookies);
    RUN_TEST(test_auth_middleware_required_auth_no_session_cookie);
    RUN_TEST(test_auth_middleware_optional_auth_no_cookies);
    RUN_TEST(test_auth_middleware_optional_auth_no_session_cookie);
    RUN_TEST(test_auth_middleware_logout_no_cookies);
    RUN_TEST(test_auth_middleware_logout_no_session_cookie);
    RUN_TEST(test_auth_middleware_type_check_admin);
    RUN_TEST(test_auth_middleware_invalid_config);
    RUN_TEST(test_auth_middleware_null_config);
    
    return UNITY_END();
}
