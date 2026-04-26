# LibAFL Sandbox

Rust + C fuzzing sandbox with Python bindings.

## Setup

Requires Rust, a C compiler (clang), and Python 3.8+.

```bash
python -m venv venv
source venv/bin/activate
pip install -r adaptive_cem/requirements.txt
```

## Build

Build the Python extension (required before running any adaptive_cem script):

```bash
maturin develop --features python
```

To build against a different PLC target (default is `boiler_plc.c`):

```bash
PLC_TARGET=pipeline_deep_oob maturin develop --features python
```

## Run

```bash
# Reference implementation (pure Python mock PLC)
python adaptive_cem/ref.py

# Rust backend, standard version
python adaptive_cem/adaptive_cem.py

# Rust backend with regime-aware ML variable selection
python adaptive_cem/adaptive_cem_kmeans.py
```

## Rust fuzzers

```bash
cargo run --release -- <strategy>
```

Available strategies: `simple_stateless`, `simple_stateful`, `pipeline_smoke`, `pipeline_greybox`
