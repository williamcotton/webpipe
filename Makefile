# Base CFLAGS from compile_flags.txt
BASE_CFLAGS = $(shell cat compile_flags.txt | tr '\n' ' ')

CC = clang
CFLAGS = -Wall -Wextra -std=c99 -g -O0 -fsanitize=address -fno-omit-frame-pointer $(BASE_CFLAGS)
LDFLAGS = -lmicrohttpd -ljansson -ldl -fsanitize=address

# Directories
PLUGIN_DIR = plugins
BUILD_DIR = build

# Main target - single wp executable in build directory
all: $(BUILD_DIR)/wp plugins

# Create build directory
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Main wp executable
$(BUILD_DIR)/wp: $(BUILD_DIR) wp.c lexer.c parser.c server.c wp.h
	$(CC) $(CFLAGS) -o $@ wp.c lexer.c parser.c server.c $(LDFLAGS)

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

# Test
test: $(BUILD_DIR)/wp
	$(BUILD_DIR)/wp -f test.wp

# Run server
run: $(BUILD_DIR)/wp install-plugins
	$(BUILD_DIR)/wp test.wp

# Clean
clean:
	rm -f wp wp_debug wp_runtime wp_runtime_debug
	rm -rf $(BUILD_DIR)
	rm -f ./plugins/*.so

.PHONY: all clean test run plugins install-plugins