# Base CFLAGS from compile_flags.txt
BASE_CFLAGS = $(shell cat compile_flags.txt | tr '\n' ' ')

CC = clang
CFLAGS = -Wall -Wextra -std=c99 -g -O0 -fno-omit-frame-pointer $(BASE_CFLAGS)
LDFLAGS = -lmicrohttpd -ljansson -ldl

# Directories
SRC_DIR = src
PLUGIN_DIR = $(SRC_DIR)/plugins
BUILD_DIR = build
TEST_DIR = test

# Main target - single wp executable in build directory
all: $(BUILD_DIR)/wp plugins

# Create build directory
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Main wp executable
$(BUILD_DIR)/wp: $(BUILD_DIR) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/wp.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(LDFLAGS) -fsanitize=address

# Debug target - single wp executable in build directory
debug: $(BUILD_DIR)/wp-debug plugins

# Debug executable
$(BUILD_DIR)/wp-debug: $(BUILD_DIR) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/wp.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(LDFLAGS)
	codesign -s - -v -f --entitlements debug.plist ./build/wp-debug

leaks: $(BUILD_DIR)/wp-debug
	leaks --atExit -- ./build/wp-debug test.wp

# Plugin targets
plugins: $(BUILD_DIR) $(BUILD_DIR)/jq.so $(BUILD_DIR)/lua.so $(BUILD_DIR)/pg.so

$(BUILD_DIR)/jq.so: $(PLUGIN_DIR)/jq.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -ljq

$(BUILD_DIR)/lua.so: $(PLUGIN_DIR)/lua.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -llua

$(BUILD_DIR)/pg.so: $(PLUGIN_DIR)/pg.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -lpq

# Install plugins to runtime directory
install-plugins: plugins
	mkdir -p ./plugins
	cp $(BUILD_DIR)/*.so ./plugins/

# Test targets
TEST_CFLAGS = $(CFLAGS) -I$(TEST_DIR) -I$(SRC_DIR) -DUNITY_INCLUDE_DOUBLE
TEST_LDFLAGS = $(LDFLAGS) -ljq -llua -lpq
TEST_COMMON_SOURCES = $(TEST_DIR)/unity/unity.c $(TEST_DIR)/helpers/test_utils.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c

# Individual test executables
$(BUILD_DIR)/test_arena: $(BUILD_DIR) $(TEST_DIR)/unit/test_arena.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_arena.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_lexer: $(BUILD_DIR) $(TEST_DIR)/unit/test_lexer.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_lexer.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_parser: $(BUILD_DIR) $(TEST_DIR)/unit/test_parser.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_parser.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_plugins: $(BUILD_DIR) $(TEST_DIR)/unit/test_plugins.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_plugins.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_jq: $(BUILD_DIR) $(TEST_DIR)/integration/test_jq.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_jq.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_lua: $(BUILD_DIR) $(TEST_DIR)/integration/test_lua.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_lua.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_pg: $(BUILD_DIR) $(TEST_DIR)/integration/test_pg.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_pg.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_pipeline: $(BUILD_DIR) $(TEST_DIR)/integration/test_pipeline.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_pipeline.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_server: $(BUILD_DIR) $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

$(BUILD_DIR)/test_e2e: $(BUILD_DIR) $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS) -lcurl

$(BUILD_DIR)/test_perf: $(BUILD_DIR) $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES) $(TEST_LDFLAGS)

# Test group targets
TEST_UNIT_BINS = $(BUILD_DIR)/test_arena $(BUILD_DIR)/test_lexer $(BUILD_DIR)/test_parser $(BUILD_DIR)/test_plugins
TEST_INTEGRATION_BINS = $(BUILD_DIR)/test_jq $(BUILD_DIR)/test_lua $(BUILD_DIR)/test_pg $(BUILD_DIR)/test_pipeline
TEST_SYSTEM_BINS = $(BUILD_DIR)/test_server $(BUILD_DIR)/test_e2e $(BUILD_DIR)/test_perf
TEST_ALL_BINS = $(TEST_UNIT_BINS) $(TEST_INTEGRATION_BINS) $(TEST_SYSTEM_BINS)

# Test commands
test: $(TEST_ALL_BINS)
	./test-runner.sh all $(TEST_ALL_BINS)

test-unit: $(TEST_UNIT_BINS)
	./test-runner.sh unit $(TEST_UNIT_BINS)

test-integration: $(TEST_INTEGRATION_BINS)
	./test-runner.sh integration $(TEST_INTEGRATION_BINS)

test-system: $(TEST_SYSTEM_BINS)
	./test-runner.sh system $(TEST_SYSTEM_BINS)

test-leaks: $(TEST_ALL_BINS)
	./test-runner.sh leaks $(TEST_ALL_BINS)

test-perf: $(BUILD_DIR)/test_perf
	@echo "Running performance tests..."
	$(BUILD_DIR)/test_perf

# Original test command
test-wp: $(BUILD_DIR)/wp
	$(BUILD_DIR)/wp -f test.wp

# Run server
run: $(BUILD_DIR)/wp install-plugins
	$(BUILD_DIR)/wp test.wp

# Clean
clean:
	rm -f wp wp_debug wp_runtime wp_runtime_debug
	rm -rf $(BUILD_DIR)
	rm -f ./plugins/*.so

.PHONY: all clean test test-unit test-integration test-system test-perf test-wp run plugins install-plugins