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

# R Detection - Multi-tier system for maximum compatibility
define detect_r_config
	@echo "Detecting R configuration..."
	@R_DETECTED=0; \
	R_HOME_DETECTED=""; \
	R_CFLAGS_DETECTED=""; \
	R_LDFLAGS_DETECTED=""; \
	R_LIBS_DETECTED=""; \
	\
	echo "Tier 1: Trying R CMD config..."; \
	if command -v R >/dev/null 2>&1; then \
		if R_HOME_TMP=$$(R RHOME 2>/dev/null); then \
			if R_CFLAGS_TMP=$$($$R_HOME_TMP/bin/R CMD config --cflags 2>/dev/null); then \
				echo "✓ R CMD config detected"; \
				R_HOME_DETECTED="$$R_HOME_TMP"; \
				R_CFLAGS_TMP_CLEAN=$$(echo "$$R_CFLAGS_TMP" | sed 's/-fopenmp//g' | sed 's/-flto[^ ]*//g'); \
				R_CFLAGS_DETECTED="$$R_CFLAGS_TMP_CLEAN -std=gnu99 -DDEFAULT_R_HOME=\\\"$$R_HOME_TMP\\\""; \
				R_LDFLAGS_TMP=$$($$R_HOME_TMP/bin/R CMD config --ldflags 2>/dev/null); \
				R_LDFLAGS_DETECTED=$$(echo "$$R_LDFLAGS_TMP" | sed 's/-fopenmp//g' | sed 's/-flto[^ ]*//g' | sed 's/-Wl[^ ]*//g'); \
				R_LIBS_TMP=$$($$R_HOME_TMP/bin/R CMD config --libs 2>/dev/null); \
				R_LIBS_DETECTED=$$(echo "$$R_LIBS_TMP" | sed 's/-fopenmp//g' | sed 's/-flto[^ ]*//g' | sed 's/-Wl[^ ]*//g'); \
				R_DETECTED=1; \
			fi; \
		fi; \
	fi; \
	\
	if [ $$R_DETECTED -eq 0 ]; then \
		echo "Tier 2: Trying common installation paths..."; \
		if [ "$(PLATFORM)" = "DARWIN" ]; then \
			for R_PATH in "/Library/Frameworks/R.framework/Resources" "/opt/homebrew/lib/R" "/usr/local/lib/R"; do \
				if [ -d "$$R_PATH/include" ] && [ -f "$$R_PATH/lib/libR.so" -o -f "$$R_PATH/lib/libR.dylib" ]; then \
					echo "✓ Found R installation at $$R_PATH"; \
					R_HOME_DETECTED="$$R_PATH"; \
					R_CFLAGS_DETECTED="-I$$R_PATH/include -std=gnu99 -DDEFAULT_R_HOME=\\\"$$R_PATH\\\""; \
					if [ "$$R_PATH" = "/Library/Frameworks/R.framework/Resources" ]; then \
						R_LDFLAGS_DETECTED="-F/Library/Frameworks -framework R"; \
					else \
						R_LDFLAGS_DETECTED="-L$$R_PATH/lib -lR"; \
					fi; \
					R_LIBS_DETECTED=""; \
					R_DETECTED=1; \
					break; \
				fi; \
			done; \
		else \
			for R_PATH in "/usr/lib/R" "/usr/local/lib/R" "/opt/R"; do \
				if [ -d "$$R_PATH/include" ] && [ -f "$$R_PATH/lib/libR.so" ]; then \
					echo "✓ Found R installation at $$R_PATH"; \
					R_HOME_DETECTED="$$R_PATH"; \
					R_CFLAGS_DETECTED="-I$$R_PATH/include -std=gnu99"; \
					R_LDFLAGS_DETECTED="-L$$R_PATH/lib"; \
					R_LIBS_DETECTED="-lR -lm"; \
					R_DETECTED=1; \
					break; \
				fi; \
			done; \
			\
			if [ $$R_DETECTED -eq 0 ] && [ -d "/usr/share/R/include" ] && [ -f "/usr/lib/R/lib/libR.so" ]; then \
				echo "✓ Found Ubuntu R installation"; \
				R_HOME_DETECTED="/usr/lib/R"; \
				R_CFLAGS_DETECTED="-I/usr/share/R/include -std=gnu99"; \
				R_LDFLAGS_DETECTED="-L/usr/lib/R/lib"; \
				R_LIBS_DETECTED="-lR -lm"; \
				R_DETECTED=1; \
			fi; \
		fi; \
	fi; \
	\
	if [ $$R_DETECTED -eq 0 ]; then \
		echo "Tier 3: Trying pkg-config..."; \
		if command -v pkg-config >/dev/null 2>&1; then \
			if pkg-config --exists libR 2>/dev/null; then \
				echo "✓ pkg-config found libR"; \
				R_CFLAGS_DETECTED=$$(pkg-config --cflags libR)" -std=gnu99"; \
				R_LDFLAGS_DETECTED=$$(pkg-config --libs libR); \
				R_LIBS_DETECTED=""; \
				R_DETECTED=1; \
			fi; \
		fi; \
	fi; \
	\
	if [ $$R_DETECTED -eq 0 ]; then \
		echo "❌ R installation not found!"; \
		echo "Please install R using one of:"; \
		if [ "$(PLATFORM)" = "DARWIN" ]; then \
			echo "  brew install r"; \
			echo "  or download from https://cran.r-project.org/bin/macosx/"; \
		else \
			echo "  sudo apt-get install r-base r-base-dev"; \
			echo "  or download from https://cran.r-project.org/bin/linux/"; \
		fi; \
		exit 1; \
	fi; \
	\
	echo "R_HOME=$$R_HOME_DETECTED" > .r-config.tmp; \
	echo "R_CFLAGS=$$R_CFLAGS_DETECTED" >> .r-config.tmp; \
	echo "R_LDFLAGS=$$R_LDFLAGS_DETECTED" >> .r-config.tmp; \
	echo "R_LIBS=$$R_LIBS_DETECTED" >> .r-config.tmp; \
	echo "✓ R configuration saved to .r-config.tmp";
endef

# Load R configuration (generate if missing)
.r-config.tmp:
	$(call detect_r_config)

-include .r-config.tmp

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
$(BUILD_DIR)/wp: $(BUILD_DIR) $(DOTENV_OBJ) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/wp.h $(SRC_DIR)/database_registry.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(DOTENV_OBJ) $(LDFLAGS) $(SANITIZE_FLAGS)

# Debug target - single wp executable in build directory
debug: $(BUILD_DIR)/wp-debug middleware

# Debug executable
$(BUILD_DIR)/wp-debug: $(BUILD_DIR) $(DOTENV_OBJ) $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(SRC_DIR)/wp.h $(SRC_DIR)/database_registry.h
	$(CC) $(CFLAGS) -o $@ $(SRC_DIR)/wp.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(DOTENV_OBJ) $(LDFLAGS) $(SANITIZE_FLAGS)
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
middleware: $(BUILD_DIR) $(BUILD_DIR)/jq.so $(BUILD_DIR)/lua.so $(BUILD_DIR)/pg.so $(BUILD_DIR)/mustache.so $(BUILD_DIR)/validate.so $(BUILD_DIR)/auth.so $(BUILD_DIR)/r.so

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

$(BUILD_DIR)/r.so: $(MIDDLEWARE_DIR)/r.c .r-config.tmp
	$(CC) $(CFLAGS) $(R_CFLAGS) -shared -fPIC -o $@ $< -ljansson $(R_LDFLAGS) $(R_LIBS)

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
TEST_COMMON_SOURCES = $(TEST_DIR)/helpers/test_utils.c $(SRC_DIR)/lexer.c $(SRC_DIR)/parser.c $(SRC_DIR)/server.c $(SRC_DIR)/database_registry.c $(DOTENV_OBJ)

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

# R test - sanitizers disabled due to conflicts with R's memory management during initialization
$(BUILD_DIR)/test_r: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_r.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_r.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_server: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_server.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_e2e: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_e2e.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS) -lcurl

$(BUILD_DIR)/test_perf: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_perf.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

# Test group targets
TEST_UNIT_BINS = $(BUILD_DIR)/test_arena $(BUILD_DIR)/test_lexer $(BUILD_DIR)/test_parser $(BUILD_DIR)/test_middleware $(BUILD_DIR)/test_cookies $(BUILD_DIR)/test_database_registry
TEST_INTEGRATION_BINS = $(BUILD_DIR)/test_jq $(BUILD_DIR)/test_lua $(BUILD_DIR)/test_mustache $(BUILD_DIR)/test_mustache_partials $(BUILD_DIR)/test_pg $(BUILD_DIR)/test_pipeline $(BUILD_DIR)/test_validate $(BUILD_DIR)/test_auth $(BUILD_DIR)/test_r
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

# Force R re-detection
detect-r:
	rm -f .r-config.tmp
	$(MAKE) .r-config.tmp

# Install required R packages for development
install-r-packages:
	@echo "Installing required R packages..."
	@if command -v R >/dev/null 2>&1; then \
		R --slave -e "if (!requireNamespace('jsonlite', quietly = TRUE)) install.packages('jsonlite', repos='https://cloud.r-project.org/')"; \
		R --slave -e "if (!requireNamespace('ggplot2', quietly = TRUE)) install.packages('ggplot2', repos='https://cloud.r-project.org/')"; \
		echo "✓ R packages installed"; \
	else \
		echo "❌ R not found. Please install R first."; \
		exit 1; \
	fi

# Clean
clean:
	rm -f wp wp_debug wp_runtime wp_runtime_debug
	rm -rf $(BUILD_DIR)
	rm -f ./middleware/*.so
	rm -f .r-config.tmp

.PHONY: all clean test test-unit test-integration test-system test-perf test-analyze test-lint test-wp run run-debug run-express-test middleware install-middleware test-leaks test-leaks-unit test-leaks-integration test-leaks-system detect-r install-r-packages