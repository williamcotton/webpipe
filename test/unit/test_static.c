#include "../unity/unity.h" 
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

// Function declarations are now in wp.h

void setUp(void) {
    // Create test public directory and files in current directory
    system("mkdir -p public/css public/images");
    system("echo 'body { margin: 0; }' > public/style.css");  
    system("echo '<html><body>Test</body></html>' > public/index.html");
    system("echo 'console.log(\"test\");' > public/app.js");
    system("echo '{\"test\": true}' > public/data.json");
    system("touch public/image.png");
    system("touch public/font.woff2");
    system("echo 'body { color: red; }' > public/css/main.css");
    system("echo 'hidden content' > public/.hidden");
    system("echo 'source code' > public/test.c");
}

void tearDown(void) {
    // Clean up test files
    system("rm -rf public");
}

// MIME Type Detection Tests
static void test_get_mime_type_web_assets(void) {
    TEST_ASSERT_STRING_EQUAL("text/html", get_mime_type("index.html"));
    TEST_ASSERT_STRING_EQUAL("text/html", get_mime_type("page.htm"));
    TEST_ASSERT_STRING_EQUAL("text/css", get_mime_type("style.css"));
    TEST_ASSERT_STRING_EQUAL("application/javascript", get_mime_type("app.js"));
    TEST_ASSERT_STRING_EQUAL("application/json", get_mime_type("data.json"));
}

static void test_get_mime_type_images(void) {
    TEST_ASSERT_STRING_EQUAL("image/png", get_mime_type("logo.png"));
    TEST_ASSERT_STRING_EQUAL("image/jpeg", get_mime_type("photo.jpg"));
    TEST_ASSERT_STRING_EQUAL("image/jpeg", get_mime_type("image.jpeg"));
    TEST_ASSERT_STRING_EQUAL("image/gif", get_mime_type("anim.gif"));
    TEST_ASSERT_STRING_EQUAL("image/svg+xml", get_mime_type("icon.svg"));
    TEST_ASSERT_STRING_EQUAL("image/webp", get_mime_type("modern.webp"));
    TEST_ASSERT_STRING_EQUAL("image/x-icon", get_mime_type("favicon.ico"));
}

static void test_get_mime_type_fonts(void) {
    TEST_ASSERT_STRING_EQUAL("font/woff", get_mime_type("font.woff"));
    TEST_ASSERT_STRING_EQUAL("font/woff2", get_mime_type("font.woff2"));
    TEST_ASSERT_STRING_EQUAL("font/ttf", get_mime_type("font.ttf"));
    TEST_ASSERT_STRING_EQUAL("font/otf", get_mime_type("font.otf"));
}

static void test_get_mime_type_unknown(void) {
    TEST_ASSERT_STRING_EQUAL("application/octet-stream", get_mime_type("unknown.xyz"));
    TEST_ASSERT_STRING_EQUAL("application/octet-stream", get_mime_type("noextension"));
    TEST_ASSERT_STRING_EQUAL("application/octet-stream", get_mime_type(NULL));
}

static void test_get_mime_type_case_insensitive(void) {
    TEST_ASSERT_STRING_EQUAL("text/css", get_mime_type("STYLE.CSS"));
    TEST_ASSERT_STRING_EQUAL("image/png", get_mime_type("Logo.PNG"));
    TEST_ASSERT_STRING_EQUAL("application/javascript", get_mime_type("App.JS"));
}

// Path Validation Tests  
static void test_validate_static_path_basic(void) {
    char safe_path[256];
    
    // Valid paths
    TEST_ASSERT_EQUAL(0, validate_static_path("/style.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/style.css", safe_path);
    
    TEST_ASSERT_EQUAL(0, validate_static_path("/index.html", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/index.html", safe_path);
    
    // Subdirectory
    TEST_ASSERT_EQUAL(0, validate_static_path("/css/main.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/css/main.css", safe_path);
}

static void test_validate_static_path_root(void) {
    char safe_path[256];
    
    // Root path should serve index.html
    TEST_ASSERT_EQUAL(0, validate_static_path("/", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/index.html", safe_path);
    
    // Empty path should also serve index.html  
    TEST_ASSERT_EQUAL(0, validate_static_path("", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/index.html", safe_path);
}

static void test_validate_static_path_security_traversal(void) {
    char safe_path[256];
    
    // Path traversal attempts
    TEST_ASSERT_EQUAL(-1, validate_static_path("/../etc/passwd", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/../../secret.txt", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/dir/../../../etc/hosts", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/subdir/../../config", safe_path, sizeof(safe_path)));
}

static void test_validate_static_path_security_hidden(void) {
    char safe_path[256];
    
    // Hidden files
    TEST_ASSERT_EQUAL(-1, validate_static_path("/.env", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/.git/config", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/.hidden", safe_path, sizeof(safe_path)));
}

static void test_validate_static_path_security_extensions(void) {
    char safe_path[256];
    
    // Dangerous extensions - exact matches at end of filename
    TEST_ASSERT_EQUAL(-1, validate_static_path("/config.wp", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/middleware.so", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/source.c", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/header.h", safe_path, sizeof(safe_path)));
    
    // But similar names should work
    TEST_ASSERT_EQUAL(0, validate_static_path("/style.css", safe_path, sizeof(safe_path))); // .css should work (not .c)
    TEST_ASSERT_EQUAL(0, validate_static_path("/app.js", safe_path, sizeof(safe_path))); // .js should work
}

static void test_validate_static_path_security_overlong(void) {
    char safe_path[256];
    
    // Create overlong path (> 255 chars)
    char long_path[300];
    memset(long_path, 'a', 299);
    long_path[0] = '/';
    long_path[299] = '\0';
    
    TEST_ASSERT_EQUAL(-1, validate_static_path(long_path, safe_path, sizeof(safe_path)));
}

static void test_validate_static_path_invalid_params(void) {
    char safe_path[256];
    
    // NULL parameters
    TEST_ASSERT_EQUAL(-1, validate_static_path(NULL, safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/test.css", NULL, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/test.css", safe_path, 0));
}

static void test_validate_static_path_windows_separators(void) {
    char safe_path[256];
    
    // Windows path separators should be blocked
    TEST_ASSERT_EQUAL(-1, validate_static_path("/dir\\file.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("\\windows\\path", safe_path, sizeof(safe_path)));
}

// File Access Validation Tests
static void test_validate_file_access_existing_files(void) {
    // These files should exist from setUp
    TEST_ASSERT_EQUAL(0, validate_file_access("./public/style.css"));
    TEST_ASSERT_EQUAL(0, validate_file_access("./public/index.html"));
    TEST_ASSERT_EQUAL(0, validate_file_access("./public/css/main.css"));
}

static void test_validate_file_access_nonexistent(void) {
    // Non-existent files should fail
    TEST_ASSERT_EQUAL(-1, validate_file_access("./public/nonexistent.txt"));
    TEST_ASSERT_EQUAL(-1, validate_file_access("./public/missing.css"));
}

static void test_validate_file_access_directory(void) {
    // Directories should be rejected
    TEST_ASSERT_EQUAL(-1, validate_file_access("./public"));
    TEST_ASSERT_EQUAL(-1, validate_file_access("./public/css"));
}

// Integration Tests combining path validation and file access
static void test_full_path_validation_workflow(void) {
    char safe_path[256];
    
    // Valid existing file
    TEST_ASSERT_EQUAL(0, validate_static_path("/style.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(0, validate_file_access(safe_path));
    
    // Valid path but non-existent file  
    TEST_ASSERT_EQUAL(-1, validate_static_path("/missing.css", safe_path, sizeof(safe_path)));
    
    // Security violation
    TEST_ASSERT_EQUAL(-1, validate_static_path("/../etc/passwd", safe_path, sizeof(safe_path)));
}

int main(void) {
    UNITY_BEGIN();
    
    // MIME Type Tests
    RUN_TEST(test_get_mime_type_web_assets);
    RUN_TEST(test_get_mime_type_images);
    RUN_TEST(test_get_mime_type_fonts);
    RUN_TEST(test_get_mime_type_unknown);
    RUN_TEST(test_get_mime_type_case_insensitive);
    
    // Path Validation Tests
    RUN_TEST(test_validate_static_path_basic);
    RUN_TEST(test_validate_static_path_root);
    RUN_TEST(test_validate_static_path_security_traversal);
    RUN_TEST(test_validate_static_path_security_hidden);
    RUN_TEST(test_validate_static_path_security_extensions);
    RUN_TEST(test_validate_static_path_security_overlong);
    RUN_TEST(test_validate_static_path_invalid_params);
    RUN_TEST(test_validate_static_path_windows_separators);
    
    // File Access Tests
    RUN_TEST(test_validate_file_access_existing_files);
    RUN_TEST(test_validate_file_access_nonexistent);
    RUN_TEST(test_validate_file_access_directory);
    
    // Integration Tests
    RUN_TEST(test_full_path_validation_workflow);
    
    return UNITY_END();
}