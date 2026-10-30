CC ?= cc
PYTHON ?= python3
CFLAGS ?= -O2 -std=c11 -Wall -Wextra -Werror
BENCH_CFLAGS ?= -O3 -std=c11 -Wall -Wextra -Werror
SANITIZER_FLAGS ?= -O2 -g -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer
BUILD_DIR := build
SYSTEM := $(shell uname -s)

.PHONY: all test sanitize benchmark smoke clean help

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
	@echo "make clean      Remove generated build files."
