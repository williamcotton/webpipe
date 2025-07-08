# Base CFLAGS from compile_flags.txt
BASE_CFLAGS = $(shell cat compile_flags.txt | tr '\n' ' ')

CC = clang
CFLAGS = -Wall -Wextra -std=c99 -O2 $(BASE_CFLAGS)
DEBUG_CFLAGS = -Wall -Wextra -std=c99 -g -O0 -fsanitize=address -fno-omit-frame-pointer $(BASE_CFLAGS)
LDFLAGS = -lmicrohttpd -ljansson -ldl
DEBUG_LDFLAGS = -lmicrohttpd -ljansson -ldl -fsanitize=address

# Directories
PLUGIN_DIR = plugins
BUILD_DIR = build

# Main targets
all: wp wp_runtime plugins

# Debug targets
debug: wp_debug wp_runtime_debug plugins_debug

# Parser/lexer tool
wp: wp.c wp.h
	$(CC) $(CFLAGS) -o wp wp.c

wp_debug: wp.c wp.h
	$(CC) $(DEBUG_CFLAGS) -o wp_debug wp.c

# Runtime server
wp_runtime: wp_runtime.c wp.h
	$(CC) $(CFLAGS) -o wp_runtime wp_runtime.c $(LDFLAGS)

wp_runtime_debug: wp_runtime.c wp.h
	$(CC) $(DEBUG_CFLAGS) -o wp_runtime_debug wp_runtime.c $(DEBUG_LDFLAGS)

# Create build directory
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Plugin targets
plugins: $(BUILD_DIR) $(BUILD_DIR)/jq.so $(BUILD_DIR)/lua.so $(BUILD_DIR)/pg.so

plugins_debug: $(BUILD_DIR) $(BUILD_DIR)/jq_debug.so $(BUILD_DIR)/lua_debug.so $(BUILD_DIR)/pg_debug.so

$(BUILD_DIR)/jq.so: $(PLUGIN_DIR)/jq.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -ljq

$(BUILD_DIR)/jq_debug.so: $(PLUGIN_DIR)/jq.c
	$(CC) $(DEBUG_CFLAGS) -shared -fPIC -o $@ $< -ljansson -ljq

$(BUILD_DIR)/lua.so: $(PLUGIN_DIR)/lua.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -llua

$(BUILD_DIR)/lua_debug.so: $(PLUGIN_DIR)/lua.c
	$(CC) $(DEBUG_CFLAGS) -shared -fPIC -o $@ $< -ljansson -llua

$(BUILD_DIR)/pg.so: $(PLUGIN_DIR)/pg.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -lpq

$(BUILD_DIR)/pg_debug.so: $(PLUGIN_DIR)/pg.c
	$(CC) $(DEBUG_CFLAGS) -shared -fPIC -o $@ $< -ljansson -lpq

# Install plugins to runtime directory
install-plugins: plugins
	mkdir -p ./plugins
	cp $(BUILD_DIR)/*.so ./plugins/

install-plugins-debug: plugins_debug
	mkdir -p ./plugins
	cp $(BUILD_DIR)/*_debug.so ./plugins/
	# Rename debug plugins to standard names for runtime
	cd ./plugins && for f in *_debug.so; do mv "$$f" "$${f%_debug.so}.so"; done

# Test
test: wp
	./wp -f test.wp

test_debug: wp_debug
	./wp_debug -f test.wp

# Clean
clean:
	rm -f wp wp_debug wp_runtime wp_runtime_debug
	rm -rf $(BUILD_DIR)
	rm -f ./plugins/*.so

.PHONY: all debug clean test test_debug plugins plugins_debug install-plugins install-plugins-debug