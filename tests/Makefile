# Makefile for Axiom SHM Test Client

CC = gcc
CFLAGS = -Wall -Wextra -O2 $(shell pkg-config --cflags wayland-client)
LDFLAGS = $(shell pkg-config --libs wayland-client)

# Protocol directory
PROTO_DIR = /usr/share/wayland-protocols
XDG_SHELL_XML = $(PROTO_DIR)/stable/xdg-shell/xdg-shell.xml

# Targets
TARGET = shm_test_client
PROTOCOL_SOURCES = xdg-shell-protocol.c
PROTOCOL_HEADERS = xdg-shell-client-protocol.h
SOURCES = shm_test_client.c $(PROTOCOL_SOURCES)
OBJECTS = $(SOURCES:.c=.o)

.PHONY: all clean

all: $(TARGET)

# Generate XDG shell protocol code
xdg-shell-protocol.c: $(XDG_SHELL_XML)
	@echo "🔧 Generating XDG shell protocol implementation..."
	wayland-scanner private-code $(XDG_SHELL_XML) $@

xdg-shell-client-protocol.h: $(XDG_SHELL_XML)
	@echo "🔧 Generating XDG shell protocol header..."
	wayland-scanner client-header $(XDG_SHELL_XML) $@

# Build object files
%.o: %.c $(PROTOCOL_HEADERS)
	@echo "🔨 Compiling $<..."
	$(CC) $(CFLAGS) -c $< -o $@

# Link the final executable
$(TARGET): $(OBJECTS)
	@echo "🔗 Linking $(TARGET)..."
	$(CC) $(OBJECTS) $(LDFLAGS) -o $@
	@echo "✅ Build complete: $(TARGET)"

clean:
	@echo "🧹 Cleaning build artifacts..."
	rm -f $(TARGET) $(OBJECTS) $(PROTOCOL_SOURCES) $(PROTOCOL_HEADERS)
	@echo "✅ Clean complete"

# Help target
help:
	@echo "Axiom SHM Test Client Build System"
	@echo "==================================="
	@echo ""
	@echo "Targets:"
	@echo "  make          - Build the test client"
	@echo "  make clean    - Remove build artifacts"
	@echo "  make help     - Show this help message"
	@echo ""
	@echo "Requirements:"
	@echo "  - wayland-client library"
	@echo "  - wayland-scanner tool"
	@echo "  - wayland-protocols package"
