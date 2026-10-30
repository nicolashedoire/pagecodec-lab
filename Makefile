CC ?= cc
CXX ?= c++
PYTHON ?= python3
CFLAGS ?= -O2 -std=c11 -Wall -Wextra -Werror
BENCH_CFLAGS ?= -O3 -std=c11 -Wall -Wextra -Werror
CXXFLAGS ?= -O3 -std=c++17 -Wall -Wextra -Werror
SANITIZER_FLAGS ?= -O2 -g -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer
BUILD_DIR := build
SYSTEM := $(shell uname -s)

.PHONY: all test sanitize benchmark smoke operations test-operations sanitize-operations end-to-end test-end-to-end sanitize-end-to-end compaction test-compaction sanitize-compaction clean help

all: test

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/codec-test: codec.c codec.h codec_test.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) codec.c codec_test.c -o $@

test: $(BUILD_DIR)/codec-test
	./$(BUILD_DIR)/codec-test

$(BUILD_DIR)/codec-test-sanitize: codec.c codec.h codec_test.c | $(BUILD_DIR)
	$(CC) $(SANITIZER_FLAGS) codec.c codec_test.c -o $@

sanitize: $(BUILD_DIR)/codec-test-sanitize
	./$(BUILD_DIR)/codec-test-sanitize

$(BUILD_DIR)/query-bench: experiments/query/query_bench.cpp | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) $< -o $@

$(BUILD_DIR)/recipe-codec.o: codec.c codec.h | $(BUILD_DIR)
	$(CC) $(BENCH_CFLAGS) -c codec.c -o $@

$(BUILD_DIR)/recipe-bench: experiments/recipe/recipe_bench.cpp $(BUILD_DIR)/recipe-codec.o
	$(CXX) $(CXXFLAGS) $^ -o $@

$(BUILD_DIR)/string-bench: experiments/strings/string_bench.cpp | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) $< -lz -o $@

operations: $(BUILD_DIR)/query-bench $(BUILD_DIR)/recipe-bench $(BUILD_DIR)/string-bench

test-operations: operations
	$(PYTHON) tests/test_query.py --binary $(abspath $(BUILD_DIR)/query-bench)
	$(PYTHON) tests/test_recipe.py --binary $(abspath $(BUILD_DIR)/recipe-bench)
	$(PYTHON) tests/test_strings.py --binary $(abspath $(BUILD_DIR)/string-bench)

sanitize-operations:
	$(MAKE) test-operations BUILD_DIR=build/operations-sanitize \
	  CXXFLAGS='-O1 -g -std=c++17 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer' \
	  BENCH_CFLAGS='-O1 -g -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer'

$(BUILD_DIR)/report-bench: experiments/end_to_end/report_bench.cpp | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) $< -o $@

end-to-end: $(BUILD_DIR)/report-bench

test-end-to-end: end-to-end
	$(PYTHON) tests/test_end_to_end.py --binary $(abspath $(BUILD_DIR)/report-bench)

sanitize-end-to-end:
	$(MAKE) test-end-to-end BUILD_DIR=build/end-to-end-sanitize \
	  CXXFLAGS='-O1 -g -std=c++17 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer'

$(BUILD_DIR)/compaction-bench: experiments/compaction/compaction_bench.cpp | $(BUILD_DIR)
	$(CXX) $(CXXFLAGS) $< -o $@

compaction: $(BUILD_DIR)/compaction-bench

test-compaction: compaction
	$(PYTHON) tests/test_compaction.py --binary $(abspath $(BUILD_DIR)/compaction-bench)

sanitize-compaction:
	$(MAKE) test-compaction BUILD_DIR=build/compaction-sanitize \
	  CXXFLAGS='-O1 -g -std=c++17 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer'

ifeq ($(SYSTEM),Darwin)
$(BUILD_DIR)/compare: compare.c codec.c codec.h | $(BUILD_DIR)
	$(CC) $(BENCH_CFLAGS) compare.c codec.c -lcompression -o $@

benchmark: $(BUILD_DIR)/compare

smoke: $(BUILD_DIR)/compare
	$(PYTHON) tests/test_benchmark.py --binary $(abspath $(BUILD_DIR)/compare)
else
benchmark smoke:
	@echo "The Apple codec comparison requires macOS and its Command Line Tools."
	@echo "The independent codec correctness tests remain available: make test sanitize."
	@exit 1
endif

clean:
	rm -rf $(BUILD_DIR)

help:
	@echo "make test       Run the portable codec correctness suite."
	@echo "make sanitize   Run the suite with AddressSanitizer and UBSan."
	@echo "make benchmark  Build the Apple codec comparator (macOS only)."
	@echo "make smoke      Validate comparator accounting on tiny fixtures (macOS only)."
	@echo "make operations Build the three operation-level experiments (C++17 + zlib)."
	@echo "make test-operations  Run deterministic correctness checks for those experiments."
	@echo "make sanitize-operations  Run those checks with ASan and UBSan."
	@echo "make test-end-to-end sanitize-end-to-end  Check the complete offline report application."
	@echo "make test-compaction sanitize-compaction  Check native compressed-segment compaction."
	@echo "make clean      Remove generated build files."
