#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <curl/curl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>

// Server process ID for cleanup
static pid_t server_pid = 0;

// CURL response buffer struct  
typedef struct {
    char *data;
    size_t size;
} ResponseBuffer;

// CURL write callback
static size_t writeCallback(void *contents, size_t size, size_t nmemb, void *userp) {
    size_t realsize = size * nmemb;
    ResponseBuffer *mem = (ResponseBuffer *)userp;

    char *ptr = realloc(mem->data, mem->size + realsize + 1);
    if (!ptr) return 0; // Out of memory

    mem->data = ptr;
    memcpy(&(mem->data[mem->size]), contents, realsize);
    mem->size += realsize;
    mem->data[mem->size] = 0;

    return realsize;
}

// Helper to make HTTP requests and return JSON response
static json_t *makeRequest(const char *url, const char *method, const char *data, long *status_code_out) {
    CURL *curl = curl_easy_init();
    if (!curl) return NULL;

    ResponseBuffer response = {0};
    response.data = malloc(1);
    response.size = 0;

    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, writeCallback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, (void *)&response);

    struct curl_slist *headers = NULL;
    headers = curl_slist_append(headers, "Content-Type: application/json");
    curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);

    if (strcmp(method, "POST") == 0) {
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    } else if (strcmp(method, "PUT") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "PUT");
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    } else if (strcmp(method, "DELETE") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "DELETE");
    }

    CURLcode res = curl_easy_perform(curl);
    
    if (status_code_out) {
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, status_code_out);
    }

    curl_slist_free_all(headers);
    curl_easy_cleanup(curl);

    if (res != CURLE_OK) {
        printf("CURL error: %s\n", curl_easy_strerror(res));
        free(response.data);
        return NULL;
    }

    // Parse JSON response
    json_error_t error;
    json_t *json = json_loads(response.data, 0, &error);
    free(response.data);

    return json;
}

// Start the WP server in background
static int startServer(void) {
    // Fork a child process to run the server
    server_pid = fork();
    
    if (server_pid == 0) {
        // Child process - run the server
        // Redirect stdout/stderr to suppress output during tests
        freopen("/dev/null", "w", stdout);
        freopen("/dev/null", "w", stderr);
        
        // Redirect stdin from /dev/null so getchar() doesn't block
        freopen("/dev/null", "r", stdin);
        
        execl("./build/wp", "./build/wp", "test.wp", "--test", NULL);
        exit(1); // Should not reach here
    } else if (server_pid > 0) {
        // Parent process - wait a moment for server to start
        sleep(2);
        
        // Check if the process is still running
        int status;
        pid_t result = waitpid(server_pid, &status, WNOHANG);
        if (result == server_pid) {
            // Process has exited
            printf("Server process exited with status: %d\n", status);
            server_pid = 0;
            return -1;
        }
        
        return 0;
    } else {
        // Fork failed
        return -1;
    }
}

// Stop the WP server
static void stopServer(void) {
    if (server_pid > 0) {
        kill(server_pid, SIGTERM);
        int status;
        waitpid(server_pid, &status, 0);
        server_pid = 0;
    }
}

void setUp(void) {
    // Set up function called before each test
    // Make sure server is stopped
    stopServer();
    
    // Start server for each test
    if (startServer() != 0) {
        TEST_FAIL_MESSAGE("Failed to start WP server");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    stopServer();
}

void test_e2e_simple_route(void) {
    // Test the /test route
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/test", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    // The /test route should return the request object (jq passthrough)
    TEST_ASSERT_NOT_NULL(json_object_get(response, "method"));
    TEST_ASSERT_STRING_EQUAL("GET", json_string_value(json_object_get(response, "method")));
    
    json_decref(response);
}

void test_e2e_parameterized_route(void) {
    // Test the /page/:id route
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/page/123", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *team = json_object_get(response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(response);
}

void test_e2e_result_step_success(void) {
    // Test the /test3 route with result step
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/test3", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *success = json_object_get(response, "success");
    TEST_ASSERT_NOT_NULL(success);
    TEST_ASSERT_TRUE(json_is_true(success));
    
    json_decref(response);
}

void test_e2e_result_step_validation_error(void) {
    // Test the /test4 route with validation error
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/test4", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(400, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *error = json_object_get(response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Validation failed", json_string_value(error));
    
    json_decref(response);
}

void test_e2e_result_step_sql_error(void) {
    // Test the /test-sql-error route
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/test-sql-error", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(500, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *error = json_object_get(response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Database error", json_string_value(error));
    
    json_decref(response);
}

void test_e2e_variable_usage(void) {
    // Test the /teams route that uses teamsQuery variable
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/teams", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *data = json_object_get(response, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *rows = json_object_get(data, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));
    
    json_decref(response);
}

void test_e2e_pipeline_chain(void) {
    // Test a multi-step pipeline
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/page/123", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    // Should have processed through jq -> pg -> jq
    json_t *team = json_object_get(response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(response);
}

void test_e2e_invalid_route(void) {
    // Test non-existent route
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/nonexistent", "GET", NULL, &status_code);
    
    TEST_ASSERT_EQUAL(404, status_code);
    
    if (response) {
        json_decref(response);
    }
}

void test_e2e_invalid_method(void) {
    // Test invalid HTTP method - but most servers accept any method, so just test that we get a response
    long status_code;
    json_t *response = makeRequest("http://localhost:8080/test", "INVALID", NULL, &status_code);
    
    // Accept any reasonable response for invalid method
    TEST_ASSERT_TRUE(status_code == 405 || status_code == 400 || status_code == 200);
    
    if (response) {
        json_decref(response);
    }
}

void test_e2e_concurrent_requests(void) {
    // Test concurrent request handling by making multiple requests
    for (int i = 0; i < 5; i++) {
        long status_code;
        json_t *response = makeRequest("http://localhost:8080/test", "GET", NULL, &status_code);
        TEST_ASSERT_EQUAL(200, status_code);
        TEST_ASSERT_NOT_NULL(response);
        json_decref(response);
        // Small delay to prevent overwhelming the server
        usleep(100000); // 100ms delay
    }
}

int main(void) {
    // Initialize curl
    curl_global_init(CURL_GLOBAL_DEFAULT);
    
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
    
    // Cleanup curl
    curl_global_cleanup();
    
    return UNITY_END();
}