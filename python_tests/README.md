# Python Tests

This directory contains Python test scripts for the LibAFL Sandbox Python bindings.

## Setup

The Python venv and library should be built from the root directory:

```bash
# From project root
python3 -m venv venv
source venv/bin/activate.fish  # on macOS/Linux
maturin develop
```

## Running Tests

```bash
# Activate venv first
source ../venv/bin/activate.fish

# Run the binding test
python test_bindings.py
```

## Test Scripts

### test_bindings.py
Basic test that verifies:
- PyTargetSession creation and booting
- Reading input size from the C program
- Reading full state size
- Reading all variable metadata from the C program

This test confirms that the Python bindings can properly access the C program's variables through the Rust library bridge.
