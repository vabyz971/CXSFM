# Build the ptrace injector (keep tools/ outside the game install dir).
#
#   make            — build inject
#   make clean
#
# The injector resolves the target's mmap/dlopen via the target's own
# link_map, so injector and game may use different libcs (NixOS vs Steam
# Runtime). Plain system cc is enough: only libc + ptrace are used.

CC      ?= cc
CFLAGS  ?= -O2 -Wall -Wextra
SRC_DIR := .
OUT_DIR := .

.PHONY: all clean

all: $(OUT_DIR)/cxsym-inject

$(OUT_DIR)/cxsym-inject: $(SRC_DIR)/inject.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f $(OUT_DIR)/cxsym-inject
