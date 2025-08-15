# Axiom Compositor Makefile
# 
# Convenient targets for development, testing, and CI workflows

# Configuration
RUST_VERSION ?= stable
CARGO := cargo
VERBOSE ?= false
HEADLESS ?= true

# Directories
SRC_DIR := src
TEST_DIR := tests
SCRIPTS_DIR := scripts
COVERAGE_DIR := target/tarpaulin

# Colors for output
BLUE := \033[0;34m
GREEN := \033[0;32m
YELLOW := \033[1;33m
RED := \033[0;31m
NC := \033[0m

# Default target
.DEFAULT_GOAL := help

.PHONY: help
help: ## Show this help message
	@echo "$(BLUE)Axiom Compositor Development Tasks$(NC)"
	@echo "=================================="
	@echo ""
	@awk 'BEGIN {FS = ":.*##"; printf "Usage: make $(GREEN)<target>$(NC)\n\nTargets:\n"} /^[a-zA-Z_-]+:.*?##/ { printf "  $(GREEN)%-15s$(NC) %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo ""
	@echo "Environment Variables:"
	@echo "  RUST_VERSION=stable  - Rust toolchain version"
	@echo "  VERBOSE=true         - Enable verbose output"
	@echo "  HEADLESS=false       - Disable headless mode"

# Build targets
.PHONY: build
build: ## Build the project
	@echo "$(BLUE)Building Axiom Compositor$(NC)"
	$(CARGO) build

.PHONY: build-release
build-release: ## Build optimized release version
	@echo "$(BLUE)Building Axiom Compositor (Release)$(NC)"
	$(CARGO) build --release

.PHONY: clean
clean: ## Clean build artifacts
	@echo "$(BLUE)Cleaning build artifacts$(NC)"
	$(CARGO) clean
	rm -rf $(COVERAGE_DIR)

# Test targets
.PHONY: test
test: ## Run all tests
	@echo "$(BLUE)Running comprehensive test suite$(NC)"
	VERBOSE=$(VERBOSE) HEADLESS=$(HEADLESS) ./$(SCRIPTS_DIR)/test.sh all

.PHONY: test-unit
test-unit: ## Run unit tests only
	@echo "$(BLUE)Running unit tests$(NC)"
	VERBOSE=$(VERBOSE) ./$(SCRIPTS_DIR)/test.sh unit

.PHONY: test-property
test-property: ## Run property-based tests
	@echo "$(BLUE)Running property-based tests$(NC)"
	VERBOSE=$(VERBOSE) ./$(SCRIPTS_DIR)/test.sh property

.PHONY: test-integration
test-integration: ## Run integration tests
	@echo "$(BLUE)Running integration tests$(NC)"
	VERBOSE=$(VERBOSE) HEADLESS=$(HEADLESS) ./$(SCRIPTS_DIR)/test.sh integration

.PHONY: test-quick
test-quick: ## Run quick test suite (lint + unit tests)
	@echo "$(BLUE)Running quick test suite$(NC)"
	VERBOSE=$(VERBOSE) ./$(SCRIPTS_DIR)/test.sh quick

.PHONY: test-ci
test-ci: ## Run CI test suite
	@echo "$(BLUE)Running CI test suite$(NC)"
	VERBOSE=$(VERBOSE) HEADLESS=$(HEADLESS) ./$(SCRIPTS_DIR)/test.sh ci

# Code quality targets
.PHONY: lint
lint: ## Run code linting (fmt + clippy)
	@echo "$(BLUE)Running code linting$(NC)"
	./$(SCRIPTS_DIR)/test.sh lint

.PHONY: fmt
fmt: ## Format code
	@echo "$(BLUE)Formatting code$(NC)"
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check: ## Check code formatting
	@echo "$(BLUE)Checking code formatting$(NC)"
	$(CARGO) fmt -- --check

.PHONY: clippy
clippy: ## Run clippy linter
	@echo "$(BLUE)Running Clippy$(NC)"
	$(CARGO) clippy --all-targets --all-features -- -D warnings

.PHONY: clippy-fix
clippy-fix: ## Apply clippy suggestions
	@echo "$(BLUE)Applying Clippy fixes$(NC)"
	$(CARGO) clippy --fix --allow-dirty --allow-staged

# Coverage targets
.PHONY: coverage
coverage: ## Generate test coverage report
	@echo "$(BLUE)Generating coverage report$(NC)"
	@chmod +x $(SCRIPTS_DIR)/coverage.sh
	./$(SCRIPTS_DIR)/coverage.sh full

.PHONY: coverage-unit
coverage-unit: ## Generate unit test coverage
	@echo "$(BLUE)Generating unit test coverage$(NC)"
	@chmod +x $(SCRIPTS_DIR)/coverage.sh
	./$(SCRIPTS_DIR)/coverage.sh unit

.PHONY: coverage-fast
coverage-fast: ## Quick coverage check
	@echo "$(BLUE)Running fast coverage check$(NC)"
	@chmod +x $(SCRIPTS_DIR)/coverage.sh
	./$(SCRIPTS_DIR)/coverage.sh fast

.PHONY: coverage-open
coverage-open: ## Open coverage report in browser
	@if [ -f "$(COVERAGE_DIR)/unit/tarpaulin-report.html" ]; then \
		echo "$(GREEN)Opening coverage report$(NC)"; \
		xdg-open $(COVERAGE_DIR)/unit/tarpaulin-report.html 2>/dev/null || \
		echo "$(YELLOW)Coverage report: file://$(shell pwd)/$(COVERAGE_DIR)/unit/tarpaulin-report.html$(NC)"; \
	else \
		echo "$(RED)Coverage report not found. Run 'make coverage' first.$(NC)"; \
	fi

# Security targets
.PHONY: audit
audit: ## Run security audit
	@echo "$(BLUE)Running security audit$(NC)"
	./$(SCRIPTS_DIR)/test.sh security

.PHONY: audit-fix
audit-fix: ## Fix security vulnerabilities
	@echo "$(BLUE)Fixing security vulnerabilities$(NC)"
	cargo audit fix

# Documentation targets
.PHONY: doc
doc: ## Generate documentation
	@echo "$(BLUE)Generating documentation$(NC)"
	$(CARGO) doc --all-features

.PHONY: doc-open
doc-open: ## Generate and open documentation
	@echo "$(BLUE)Generating and opening documentation$(NC)"
	$(CARGO) doc --all-features --open

.PHONY: test-doc
test-doc: ## Run documentation tests
	@echo "$(BLUE)Running documentation tests$(NC)"
	$(CARGO) test --doc

# Benchmark targets
.PHONY: bench
bench: ## Run benchmarks
	@echo "$(BLUE)Running benchmarks$(NC)"
	@if [ -d "benches" ]; then \
		HEADLESS=$(HEADLESS) $(CARGO) bench; \
	else \
		echo "$(YELLOW)No benchmark directory found$(NC)"; \
	fi

# Development convenience targets
.PHONY: check
check: fmt-check clippy test-unit ## Quick development check (format, lint, unit tests)

.PHONY: pre-commit
pre-commit: fmt clippy test-quick ## Pre-commit hook (format, lint, quick tests)

.PHONY: install-deps
install-deps: ## Install development dependencies
	@echo "$(BLUE)Installing development dependencies$(NC)"
	cargo install cargo-tarpaulin || true
	cargo install cargo-audit || true
	cargo install cargo-license || true

.PHONY: setup
setup: install-deps ## Setup development environment
	@echo "$(BLUE)Setting up development environment$(NC)"
	@chmod +x $(SCRIPTS_DIR)/*.sh
	@echo "$(GREEN)Development environment setup complete$(NC)"

# CI/CD targets
.PHONY: ci-local
ci-local: ## Simulate CI pipeline locally
	@echo "$(BLUE)Simulating CI pipeline locally$(NC)"
	$(MAKE) clean
	$(MAKE) check
	$(MAKE) test-ci
	$(MAKE) coverage-unit
	$(MAKE) audit

# Release targets
.PHONY: release-check
release-check: ## Validate release readiness
	@echo "$(BLUE)Checking release readiness$(NC)"
	$(MAKE) clean
	$(MAKE) test
	$(MAKE) build-release
	$(CARGO) package --dry-run

# Maintenance targets
.PHONY: update
update: ## Update dependencies
	@echo "$(BLUE)Updating dependencies$(NC)"
	$(CARGO) update

.PHONY: outdated
outdated: ## Check for outdated dependencies
	@echo "$(BLUE)Checking for outdated dependencies$(NC)"
	@if command -v cargo-outdated >/dev/null 2>&1; then \
		cargo outdated; \
	else \
		echo "$(YELLOW)cargo-outdated not installed. Install with: cargo install cargo-outdated$(NC)"; \
	fi

# Statistics and info
.PHONY: stats
stats: ## Show project statistics
	@echo "$(BLUE)Project Statistics$(NC)"
	@echo "==================="
	@echo "Lines of code:"
	@find $(SRC_DIR) -name "*.rs" -exec wc -l {} + | tail -1
	@echo ""
	@echo "Test files:"
	@find $(SRC_DIR) -name "*test*.rs" -o -name "tests.rs" | wc -l
	@find $(TEST_DIR) -name "*.rs" 2>/dev/null | wc -l || echo "0"
	@echo ""
	@echo "Dependencies:"
	@grep -E "^[a-zA-Z]" Cargo.toml | grep -v "^\[" | wc -l

.PHONY: tree
tree: ## Show project structure
	@echo "$(BLUE)Project Structure$(NC)"
	@tree -I 'target|.git' || find . -type f -name "*.rs" | head -20

# Create required directories
$(SCRIPTS_DIR):
	mkdir -p $(SCRIPTS_DIR)

$(COVERAGE_DIR):
	mkdir -p $(COVERAGE_DIR)

# Ensure scripts are executable
$(SCRIPTS_DIR)/%.sh: | $(SCRIPTS_DIR)
	chmod +x $@
