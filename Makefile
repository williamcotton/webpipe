# Platform detection
PLATFORM := $(shell sh -c 'uname -s 2>/dev/null | tr 'a-z' 'A-Z'')

# Base CFLAGS from compile_flags.txt
BASE_CFLAGS = $(shell cat compile_flags.txt | tr '\n' ' ')

# Platform-specific settings
ifeq ($(PLATFORM),LINUX)
	CC = clang
	LUA_LIB = -llua5.4
	LUA_INCLUDE = -I/usr/include/lua5.4
	PG_LIBDIR = /usr/lib/x86_64-linux-gnu
	PG_INCLUDE = -I/usr/include/postgresql
	SANITIZE_FLAGS = -fsanitize=address,undefined
	PLATFORM_LIBS = -lm -lpthread -ldl
	CODESIGN_CMD = 
	TIDY = clang-tidy
else ifeq ($(PLATFORM),DARWIN)
	CC = clang
	LUA_LIB = -llua
	LUA_INCLUDE = -I/opt/homebrew/include/lua
	PG_LIBDIR = /opt/homebrew/lib/postgresql@14
	PG_INCLUDE = -I/opt/homebrew/include/postgresql@14
	SANITIZE_FLAGS = -fsanitize=address,undefined
	PLATFORM_LIBS = -ldl
	CODESIGN_CMD = codesign -s - -v -f --entitlements debug.plist
	TIDY = $(shell brew --prefix llvm)/bin/clang-tidy
endif

# Dotenv-c integration
DOTENV_SRC = $(DEPS_DIR)/dotenv-c/dotenv.c
DOTENV_OBJ = $(BUILD_DIR)/dotenv.o
DOTENV_INCLUDE = -I$(DEPS_DIR)/dotenv-c

# Common flags
CFLAGS = -Wall -Wextra -std=c99 -g -O0 -fno-omit-frame-pointer $(BASE_CFLAGS) $(LUA_INCLUDE) $(PG_INCLUDE) $(DOTENV_INCLUDE)
LDFLAGS = -lmicrohttpd -ljansson $(LUA_LIB) -L$(PG_LIBDIR) -lpq $(PLATFORM_LIBS)

# Directories
SRC_DIR = src
MIDDLEWARE_DIR = $(SRC_DIR)/middleware
BUILD_DIR = build
TEST_DIR = test
DEPS_DIR = deps

# Main target - single wp executable in build directory
all: $(BUILD_DIR)/wp middleware

# Create build directory
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Dotenv-c object file
$(DOTENV_OBJ): $(BUILD_DIR) $(DOTENV_SRC)
	$(CC) $(CFLAGS) -c -o $@ $(DOTENV_SRC)

# Main wp executable
$(BUILD_DIR)/wp: $(BUILD_DIR) $(DOTENV_OBJ) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/testing.c $(SRC_DIR)/wp.h $(SRC_DIR)/database_registry.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/testing.c $(DOTENV_OBJ) $(LDFLAGS) $(SANITIZE_FLAGS)

# Debug target - single wp executable in build directory
debug: $(BUILD_DIR)/wp-debug middleware

# Debug executable
$(BUILD_DIR)/wp-debug: $(BUILD_DIR) $(DOTENV_OBJ) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/testing.c $(SRC_DIR)/wp.h $(SRC_DIR)/database_registry.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/testing.c $(DOTENV_OBJ) $(LDFLAGS) $(SANITIZE_FLAGS)
ifneq ($(CODESIGN_CMD),)
	$(CODESIGN_CMD) ./build/wp-debug
endif

leaks: $(BUILD_DIR)/wp-debug
ifeq ($(PLATFORM),LINUX)
	valgrind --tool=memcheck --leak-check=full --error-exitcode=1 --num-callers=30 -s ./build/wp-debug test.wp
else ifeq ($(PLATFORM),DARWIN)
	leaks --atExit -- ./build/wp-debug test.wp
endif

# Middleware targets
middleware: $(BUILD_DIR) $(BUILD_DIR)/jq.so $(BUILD_DIR)/lua.so $(BUILD_DIR)/pg.so $(BUILD_DIR)/mustache.so $(BUILD_DIR)/validate.so $(BUILD_DIR)/auth.so $(BUILD_DIR)/cache.so $(BUILD_DIR)/log.so

$(BUILD_DIR)/jq.so: $(MIDDLEWARE_DIR)/jq.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -ljq

$(BUILD_DIR)/lua.so: $(MIDDLEWARE_DIR)/lua.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson $(LUA_LIB)

$(BUILD_DIR)/pg.so: $(MIDDLEWARE_DIR)/pg.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -lpq

$(BUILD_DIR)/mustache.so: $(MIDDLEWARE_DIR)/mustache.c deps/mustach/mustach.c deps/mustach/mustach-jansson.c deps/mustach/mustach-wrap.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $^ -ljansson

$(BUILD_DIR)/validate.so: $(MIDDLEWARE_DIR)/validate.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson

$(BUILD_DIR)/auth.so: $(MIDDLEWARE_DIR)/auth.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -largon2

$(BUILD_DIR)/cache.so: $(MIDDLEWARE_DIR)/cache.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson

$(BUILD_DIR)/log.so: $(MIDDLEWARE_DIR)/log.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson

# Install middleware to runtime directory
install-middleware: middleware
	mkdir -p ./middleware
	cp $(BUILD_DIR)/*.so ./middleware/

# Install middleware API header for third-party development
install-api-header:
	mkdir -p ./include
	cp ./include/webpipe-middleware-api.h ./include/

# Test targets
TEST_CFLAGS = $(CFLAGS) -I$(TEST_DIR) -I$(SRC_DIR) -DUNITY_INCLUDE_DOUBLE
TEST_LDFLAGS = $(LDFLAGS) -ljq
# Unity framework with suppressed warnings
UNITY_CFLAGS = $(CFLAGS) -I$(TEST_DIR) -I$(SRC_DIR) -DUNITY_INCLUDE_DOUBLE -Wno-double-promotion
TEST_COMMON_SOURCES = $(TEST_DIR)/helpers/test_utils.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/testing.c $(DOTENV_OBJ)

# Unity object file with suppressed warnings
$(BUILD_DIR)/unity.o: $(BUILD_DIR) $(TEST_DIR)/unity/unity.c
	$(CC) $(UNITY_CFLAGS) -c -o $@ $(TEST_DIR)/unity/unity.c

# Individual test executables
$(BUILD_DIR)/test_arena: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_arena.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_arena.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_lexer: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_lexer.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_lexer.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_parser: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_parser.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_parser.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_middleware: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_middleware.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_middleware.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_cookies: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_cookies.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_cookies.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_database_registry: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_database_registry.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_database_registry.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_testing: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_testing.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_testing.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_jq: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_jq.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_jq.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_lua: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_lua.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_lua.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_mustache: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_mustache.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_mustache.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_mustache_partials: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_mustache_partials.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_mustache_partials.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_pg: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_pg.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_pg.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_pipeline: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_pipeline.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_pipeline.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_validate: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_validate.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_validate.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_auth: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_auth.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_auth.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_cache: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_cache.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_cache.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_log: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_log.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_log.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_server: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_e2e: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS) -lcurl

$(BUILD_DIR)/test_perf: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

# Test group targets
TEST_UNIT_BINS = $(BUILD_DIR)/test_arena $(BUILD_DIR)/test_lexer $(BUILD_DIR)/test_parser $(BUILD_DIR)/test_middleware $(BUILD_DIR)/test_cookies $(BUILD_DIR)/test_database_registry $(BUILD_DIR)/test_testing
TEST_INTEGRATION_BINS = $(BUILD_DIR)/test_jq $(BUILD_DIR)/test_lua $(BUILD_DIR)/test_mustache $(BUILD_DIR)/test_mustache_partials $(BUILD_DIR)/test_pg $(BUILD_DIR)/test_pipeline $(BUILD_DIR)/test_validate $(BUILD_DIR)/test_auth $(BUILD_DIR)/test_cache $(BUILD_DIR)/test_log
TEST_SYSTEM_BINS = $(BUILD_DIR)/test_server $(BUILD_DIR)/test_e2e $(BUILD_DIR)/test_perf
TEST_ALL_BINS = $(TEST_UNIT_BINS) $(TEST_INTEGRATION_BINS) $(TEST_SYSTEM_BINS)

# Test commands
test: $(TEST_ALL_BINS) install-middleware
	./test-runner.sh all $(TEST_ALL_BINS)

test-unit: $(TEST_UNIT_BINS)
	./test-runner.sh unit $(TEST_UNIT_BINS)

test-integration: $(TEST_INTEGRATION_BINS)
	./test-runner.sh integration $(TEST_INTEGRATION_BINS)

test-system: $(TEST_SYSTEM_BINS)
	./test-runner.sh system $(TEST_SYSTEM_BINS)

test-bdd-suite: $(BUILD_DIR)/wp install-middleware
	@echo "Running BDD test suite..."
	./build/wp test.wp --test

test-leaks: $(TEST_ALL_BINS)
	./test-runner.sh leaks $(TEST_ALL_BINS)

test-leaks-unit: $(TEST_UNIT_BINS)
	./test-runner.sh leaks $(TEST_UNIT_BINS)

test-leaks-integration: $(TEST_INTEGRATION_BINS)
	./test-runner.sh leaks $(TEST_INTEGRATION_BINS)

test-leaks-system: $(BUILD_DIR)/test_server $(BUILD_DIR)/test_perf
	./test-runner.sh leaks $(BUILD_DIR)/test_server $(BUILD_DIR)/test_perf

test-perf: $(BUILD_DIR)/test_perf
	@echo "Running performance tests..."
	$(BUILD_DIR)/test_perf

test-analyze:
	clang --analyze $(SRC_DIR)/*.c $(MIDDLEWARE_DIR)/*.c $(CFLAGS) -Xanalyzer -analyzer-output=text -Xanalyzer -analyzer-checker=core,deadcode,nullability,optin,osx,security,unix,valist -Xanalyzer -analyzer-disable-checker -Xanalyzer security.insecureAPI.DeprecatedOrUnsafeBufferHandling -Werror

test-lint:
ifeq ($(PLATFORM),LINUX)
	$(TIDY) --checks=-clang-analyzer-security.insecureAPI.DeprecatedOrUnsafeBufferHandling,-clang-diagnostic-unused-command-line-argument,-clang-diagnostic-disabled-macro-expansion -warnings-as-errors=* $(SRC_DIR)/*.c $(MIDDLEWARE_DIR)/*.c -- $(CFLAGS) $(SANITIZE_FLAGS)
else ifeq ($(PLATFORM),DARWIN)
	$(TIDY) --checks=-clang-analyzer-security.insecureAPI.DeprecatedOrUnsafeBufferHandling,-clang-diagnostic-unused-command-line-argument,-clang-diagnostic-disabled-macro-expansion -warnings-as-errors=* $(SRC_DIR)/*.c $(MIDDLEWARE_DIR)/*.c -- $(CFLAGS) $(SANITIZE_FLAGS)
endif

# Run server
run: $(BUILD_DIR)/wp install-middleware
	$(BUILD_DIR)/wp test.wp

# Run server with debug binary (AddressSanitizer enabled)
run-debug: $(BUILD_DIR)/wp-debug install-middleware
	$(BUILD_DIR)/wp-debug test.wp --port 8085

# Clean
clean:
	rm -f wp wp_debug wp_runtime wp_runtime_debug
	rm -rf $(BUILD_DIR)
	rm -f ./middleware/*.so

.PHONY: all clean test test-unit test-integration test-system test-bdd-suite test-perf test-analyze test-lint test-wp run run-debug run-express-test middleware install-middleware test-leaks test-leaks-unit test-leaks-integration test-leaks-system