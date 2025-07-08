# Base CFLAGS from compile_flags.txt
BASE_CFLAGS = $(shell cat compile_flags.txt | tr '\n' ' ')

CC = clang
CFLAGS = -Wall -Wextra -std=c99 -O2 $(BASE_CFLAGS)
LDFLAGS = -lmicrohttpd -ljansson -ldl

# Directories
PLUGIN_DIR = plugins
BUILD_DIR = build

# Main targets
all: wp wp_runtime plugins

# Parser/lexer tool
wp: wp.c wp.h
	$(CC) $(CFLAGS) -o wp wp.c

# Runtime server
wp_runtime: wp_runtime.c wp.h
	$(CC) $(CFLAGS) -o wp_runtime wp_runtime.c $(LDFLAGS)

# Create build directory
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

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
test: wp
	./wp -f test.wp

# Clean
clean:
	rm -f wp wp_runtime
	rm -rf $(BUILD_DIR)
	rm -f ./plugins/*.so

.PHONY: all clean test plugins install-plugins