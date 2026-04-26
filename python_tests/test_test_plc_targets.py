#!/usr/bin/env python3
"""Build and validate basic test PLC targets via Python bindings."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ["test_plc_add", "test_plc_counter", "test_plc_scan"]


def run_cmd(cmd: list[str], env: dict[str, str] | None = None) -> None:
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def verify_common(session) -> None:
    session.boot()
    session.reset()

    metadata = session.var_metadata()
    require(len(metadata) > 0, "expected non-empty metadata")

    input_hints = session.input_hints()
    require(len(input_hints) == session.input_size(), "input_hints count should match input_size")

    values = session.read_vars()
    require(isinstance(values, dict), "read_vars should return a dict")
    require(len(values) == len(metadata), "value count should match metadata count")

    raw_state = session.state()
    require(session.set_state(raw_state), "set_state round-trip should succeed")


def verify_add(session) -> None:
    verify_common(session)

    session.step(bytes([7, 5]))
    values = session.read_vars(["a", "b", "sum", "steps", "status"])
    require(values["a"] == 7, "a should be 7")
    require(values["b"] == 5, "b should be 5")
    require(values["sum"] == 12, "sum should be 12")
    require(values["steps"] == 1, "steps should be 1")
    require(values["status"] is True, "status should be true")

    session.write_vars({"sum": 99})
    require(session.read_vars(["sum"])["sum"] == 99, "write_vars should update sum")


def verify_counter(session) -> None:
    verify_common(session)

    session.write_vars({"limit": 6, "counter": 0})
    session.step(bytes([2]))
    session.step_series(bytes([2, 3]), 1)

    values = session.read_vars(["counter", "limit", "steps", "reached"])
    require(values["counter"] == 7, "counter should be 7")
    require(values["limit"] == 6, "limit should be 6")
    require(values["steps"] == 3, "steps should be 3")
    require(values["reached"] is True, "reached should be true")


def verify_scan(session) -> None:
    verify_common(session)

    session.step(bytes([1]))
    values1 = session.read_vars(["scan_cycle", "phase", "latched", "status"])
    require(values1["scan_cycle"] == 1, "scan_cycle should be 1")
    require(values1["latched"] == 1, "latched should be 1")
    require(values1["status"] is True, "status should be true")

    session.step_series(bytes([2, 0, 1]), 1)
    values2 = session.read_vars(["scan_cycle", "phase", "latched", "accumulator"])
    require(values2["scan_cycle"] == 4, "scan_cycle should be 4")
    require(values2["latched"] == 1, "latched should be 1")
    require(values2["accumulator"] == 4, "accumulator should be 4")


VERIFY_BY_TARGET: dict[str, Callable] = {
    "test_plc_add": verify_add,
    "test_plc_counter": verify_counter,
    "test_plc_scan": verify_scan,
}


def verify_target(target: str) -> None:
    from libafl_sandbox import TargetSession

    session = TargetSession()
    VERIFY_BY_TARGET[target](session)


def build_target(target: str) -> None:
    env = os.environ.copy()
    env["PLC_TARGET"] = target
    run_cmd(["maturin", "develop"], env=env)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()

    targets = [args.target] if args.target else TARGETS

    if args.verify_only:
        verify_target(targets[0])
        return 0

    for target in targets:
        print(f"\n=== Running target: {target} ===")

        build_target(target)
        run_cmd([sys.executable, __file__, "--target", target, "--verify-only"])
        print(f"OK: {target}")

    print("\nAll target checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
