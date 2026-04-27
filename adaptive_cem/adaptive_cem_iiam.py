#!/usr/bin/env python3
"""
Hybrid Go-Explore + CEM with ML Deduction, Heuristic Pruning, and Stagnation Breaking.

Extended with Technique 2 — Interventional Input Attribution Maps (IIAM):

The baseline CEM treats all 7 input bytes uniformly, maintaining independent Gaussians
for every byte at every timestep. But the PLC's logic makes this wasteful: in PRIME phase,
only pump_rate (byte 0), back_pressure (byte 3), and valve_pos (byte 1) affect prime_score;
the other three analog bytes have exactly zero causal effect and waste 3/6 of the CEM's
sampling budget. In FILL zones, only specific (pump+valve, pipe_temp) windows matter per
zone, while feed_conc and coolant_rate are irrelevant.

IIAM computes a numerical Jacobian of the target variable with respect to each analog input
byte via paired interventional rollouts: for each of the 6 analog bytes, two extra rollouts
are run — one with that byte perturbed +δ across all timesteps, one with -δ. The central
difference in the maximum target-variable value achieved over the horizon gives the causal
attribution of that byte from the current parent state.

Attribution is converted to per-byte standard deviations for the CEM:
  - High attribution → narrow std (CEM focuses on this byte's effective range)
  - Zero attribution → wide std (explore freely; byte has no effect here)

The overhead is 2 × ANALOG_DIMS = 12 extra rollouts per outer CEM call, paid once before
generation 0. Results are cached by (phase, fill_head_zone) so repeated selection of the
same regime costs nothing.

No Rust or C changes are required. IIAM uses only the existing rollout_states_batch and
decode_raw_states_batch primitives.
"""

from __future__ import annotations

import argparse
import copy
import math
import random
import statistics
from collections import deque
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np
import pandas as pd
from sklearn.ensemble import RandomForestRegressor

from libafl_sandbox import TargetSession

ACTION_NAMES = [
    "pump_rate",
    "valve_pos",
    "pipe_temp",
    "back_pressure",
    "feed_conc",
    "coolant_rate",
    "cmd",
]
ANALOG_DIMS = 6
ACTION_LOW = [0, 0, 0, 0, 0, 0, 0]
ACTION_HIGH = [255, 255, 255, 255, 255, 255, 1]

# IIAM hyper-parameters
ATTR_DELTA: int = 16        # perturbation magnitude (~6% of 0-255 range)
ATTR_STD_MIN: float = 6.0   # std floor for the byte with highest attribution
ATTR_STD_MAX: float = 90.0  # std ceiling for bytes with zero attribution (baseline default)


class RustPipelinePLC:
    """Adapter that exposes the same interface expected by the search driver."""

    def __init__(self) -> None:
        self.session = TargetSession()
        self.session.boot()
        self.reset_state()

    def reset_state(self) -> None:
        self.session.reset()

    def get_state(self) -> Dict[str, Any]:
        values = self.session.read_vars()
        values["_raw_state"] = self.session.state()
        return values

    def set_state(self, state: Dict[str, Any]) -> None:
        raw_state = state.get("_raw_state")
        if isinstance(raw_state, (bytes, bytearray)):
            self.session.set_state(bytes(raw_state))
            return

        writable = {k: v for k, v in state.items() if not k.startswith("_")}
        if writable:
            self.session.write_vars(writable)

    def scan(self, action: Sequence[int]) -> Tuple[Dict[str, Any], bool]:
        action = [max(l, min(h, int(v))) for v, l, h in zip(action, ACTION_LOW, ACTION_HIGH)]
        self.session.step(bytes(action))
        state = self.get_state()
        terminated = int(state.get("fill_head", 0)) >= 64
        return state, terminated

    def decode_raw_state(self, raw_state: bytes) -> Dict[str, Any]:
        state_arr = np.frombuffer(raw_state, dtype=np.uint8)[np.newaxis, :]
        result = self.session.decode_states_batch(np.ascontiguousarray(state_arr))
        result[0]["_raw_state"] = raw_state
        return result[0]

    def decode_raw_states_batch(self, raw_states: np.ndarray) -> List[Dict[str, Any]]:
        """Decode a (..., state_size) uint8 array into N dicts without touching global PLC state."""
        flat = np.ascontiguousarray(raw_states.reshape(-1, raw_states.shape[-1]))
        decoded = self.session.decode_states_batch(flat)
        for i, row in enumerate(raw_states.reshape(-1, raw_states.shape[-1])):
            decoded[i]["_raw_state"] = row.tobytes()
        return decoded

    def rollout_states_batch(self, initial_state: bytes, actions: np.ndarray) -> np.ndarray:
        if actions.ndim != 3:
            raise ValueError(
                f"actions must be a 3D array shaped (rollouts, steps, input_size), got ndim={actions.ndim}"
            )

        clipped = np.clip(actions, np.array(ACTION_LOW, dtype=np.int16), np.array(ACTION_HIGH, dtype=np.int16))
        action_u8 = np.ascontiguousarray(clipped.astype(np.uint8, copy=False))

        state_vec = np.frombuffer(initial_state, dtype=np.uint8)
        if state_vec.size != self.session.state_size():
            raise ValueError(
                f"initial_state size mismatch: got {state_vec.size}, expected {self.session.state_size()}"
            )

        initial_states = np.repeat(state_vec[np.newaxis, :], action_u8.shape[0], axis=0)
        initial_states = np.ascontiguousarray(initial_states, dtype=np.uint8)
        return self.session.rollout_states_batch(initial_states, action_u8)


class InputAttributor:
    """
    Computes interventional input attribution maps for the CEM's analog input bytes.

    For each of the ANALOG_DIMS (6) analog bytes, two rollouts are run from the current
    parent state: one with that byte perturbed +ATTR_DELTA at every timestep, one with
    -ATTR_DELTA. The central-difference change in the maximum target-variable value over
    the horizon is the causal attribution of that byte.

    Attribution is then mapped to per-byte CEM standard deviations:
      - Byte with peak attribution → std = ATTR_STD_MIN (tight; CEM exploits valid range)
      - Byte with zero attribution → std = ATTR_STD_MAX (wide; explore freely)
      - Intermediate attribution  → linearly interpolated std

    Results are cached by (phase, fill_head_zone). The cache is cleared when it exceeds
    64 entries (32 unique phase×zone combinations × 2 for safety).
    """

    def __init__(self, target_var: str) -> None:
        self.target_var = target_var
        self._cache: Dict[Tuple[int, int], np.ndarray] = {}

    def _regime_key(self, state: Dict[str, Any]) -> Tuple[int, int]:
        phase = int(state.get("phase", 0))
        fill_head = int(state.get(self.target_var, 0))
        return (phase, fill_head // 8)

    def _build_perturbed_batch(
        self, ref_action: np.ndarray
    ) -> np.ndarray:
        """
        Build the 12-rollout attribution batch from a reference action sequence.

        ref_action: (H, 7) uint8 array — the reference action for all timesteps.

        Returns: (2*ANALOG_DIMS, H, 7) uint8 array.
          Row 2b:   ref with byte b += ATTR_DELTA at every timestep (clipped to ACTION_HIGH)
          Row 2b+1: ref with byte b -= ATTR_DELTA at every timestep (clipped to ACTION_LOW)
        """
        H = ref_action.shape[0]
        n_rollouts = 2 * ANALOG_DIMS

        # Work in int16 to avoid uint8 overflow before clipping
        batch = np.tile(ref_action.astype(np.int16), (n_rollouts, 1, 1))  # (12, H, 7)

        for b in range(ANALOG_DIMS):
            batch[2 * b,     :, b] += ATTR_DELTA
            batch[2 * b + 1, :, b] -= ATTR_DELTA

        low = np.array(ACTION_LOW,  dtype=np.int16)
        high = np.array(ACTION_HIGH, dtype=np.int16)
        return np.clip(batch, low, high).astype(np.uint8)

    def compute(
        self,
        plc: RustPipelinePLC,
        raw_parent: bytes,
        ref_action: np.ndarray,
        horizon: int,
    ) -> np.ndarray:
        """
        Run 12 interventional rollouts and return an ANALOG_DIMS-length std array.

        The cost is 12 rollouts — about 3% of a normal CEM generation (384 rollouts).
        """
        perturbed = self._build_perturbed_batch(ref_action)         # (12, H, 7)
        raw_states = plc.rollout_states_batch(raw_parent, perturbed) # (12, H, state_size)
        decoded = plc.decode_raw_states_batch(raw_states)            # list of 12*H dicts

        attribution = np.zeros(ANALOG_DIMS, dtype=np.float32)
        for b in range(ANALOG_DIMS):
            # Maximum target_var observed over the horizon for +delta and -delta rollouts.
            plus_max = max(
                float(decoded[(2 * b) * horizon + t].get(self.target_var, 0.0))
                for t in range(horizon)
            )
            minus_max = max(
                float(decoded[(2 * b + 1) * horizon + t].get(self.target_var, 0.0))
                for t in range(horizon)
            )
            attribution[b] = abs(plus_max - minus_max) / (2.0 * ATTR_DELTA)

        peak = float(attribution.max())
        if peak < 1e-6:
            # No byte moved the target from this state; keep default wide std.
            return np.full(ANALOG_DIMS, ATTR_STD_MAX, dtype=np.float32)

        attr_norm = attribution / peak  # [0, 1], peak byte = 1.0
        # High attribution (norm ≈ 1) → narrow std; zero attribution → wide std.
        std_arr = ATTR_STD_MAX - (ATTR_STD_MAX - ATTR_STD_MIN) * attr_norm
        return std_arr.clip(ATTR_STD_MIN, ATTR_STD_MAX).astype(np.float32)

    def get_or_compute(
        self,
        plc: RustPipelinePLC,
        parent: "ArchiveEntry",
        ref_action: np.ndarray,
        horizon: int,
    ) -> np.ndarray:
        """
        Return cached attribution std for this regime, computing it if not yet cached.
        """
        key = self._regime_key(parent.state)
        if key not in self._cache:
            if len(self._cache) > 64:
                self._cache.clear()

            raw_parent = parent.state.get("_raw_state")
            if not isinstance(raw_parent, (bytes, bytearray)):
                plc.set_state(parent.state)
                raw_parent = plc.session.state()

            self._cache[key] = self.compute(plc, bytes(raw_parent), ref_action, horizon)

        return self._cache[key]

    def log_summary(self, state: Dict[str, Any]) -> None:
        """Print a human-readable attribution summary for the current regime."""
        key = self._regime_key(state)
        if key not in self._cache:
            return
        std_arr = self._cache[key]
        phase_names = {0: "IDLE", 1: "PRIME", 2: "FLOW", 3: "FILL"}
        phase_str = phase_names.get(key[0], str(key[0]))
        entries = ", ".join(
            f"{ACTION_NAMES[b]}={std_arr[b]:.1f}"
            for b in range(ANALOG_DIMS)
        )
        print(f"  [IIAM] {phase_str}/zone{key[1]}: {entries}")


class ProgressVariableDiscoverer:
    def __init__(self, target_var: str, max_vars: int = 5):
        self.target_var = target_var
        self.max_vars = max_vars
        self.data_buffer = deque(maxlen=15000)
        self.target_buffer = deque(maxlen=15000)

    def add_rollout(self, states: List[Dict[str, Any]], final_target: float):
        for s in states[::5]:
            self.data_buffer.append({k: v for k, v in s.items() if is_scalar(v)})
            self.target_buffer.append(final_target)

    def get_valid_variables(self, df: pd.DataFrame) -> List[str]:
        valid_cols = []

        variances = df.var()
        moving_cols = variances[variances > 1e-5].index.tolist()

        time_index = pd.Series(np.arange(len(df)))

        for col in moving_cols:
            if col == self.target_var:
                continue

            correlation_with_time = abs(df[col].corr(time_index))
            diffs = df[col].diff().dropna()
            step_variance = diffs.var()

            is_counter = correlation_with_time > 0.98 or step_variance < 1e-5

            if not is_counter:
                valid_cols.append(col)

        return valid_cols

    def analyze(self, current_vars: List[str], force_random: bool = False) -> List[str]:
        if len(self.data_buffer) < 200 and not force_random:
            return current_vars

        df = pd.DataFrame(self.data_buffer).fillna(0)
        valid_cols = self.get_valid_variables(df)

        if not valid_cols:
            if force_random:
                print("  -> [STAGNATION DETECTED] No valid variables survived heuristics; keeping current set.")
            return current_vars

        if force_random:
            print("  -> [STAGNATION DETECTED] Bypassing ML. Injecting random variables to explore new state spaces.")
            k = min(self.max_vars, len(valid_cols))
            current_set = set(current_vars)
            preferred = [col for col in valid_cols if col not in current_set]

            if not preferred:
                print("  -> [STAGNATION DETECTED] No alternative variables available; keeping current set.")
                return current_vars

            if len(preferred) >= k:
                return random.sample(preferred, k)

            chosen = preferred[:]
            needed = k - len(chosen)
            fallback_pool = [col for col in valid_cols if col not in chosen]
            if needed > 0 and fallback_pool:
                chosen.extend(random.sample(fallback_pool, min(needed, len(fallback_pool))))
            return chosen

        y = np.array(self.target_buffer)
        if np.std(y) < 1e-5:
            variances = df[valid_cols].var().sort_values(ascending=False)
            return variances.index[: self.max_vars].tolist()

        X = df[valid_cols]
        rf = RandomForestRegressor(n_estimators=50, max_depth=6, random_state=42, n_jobs=-1)
        rf.fit(X, y)

        importances = rf.feature_importances_
        sorted_idx = np.argsort(importances)[::-1]

        best_vars = []
        for idx in sorted_idx:
            if importances[idx] > 0.015:
                best_vars.append(valid_cols[idx])
                if len(best_vars) >= self.max_vars:
                    break

        return best_vars if best_vars else current_vars


@dataclass
class SearchConfig:
    target_var: str = "fill_head"
    target_goal: float = 64.0
    discovery_interval: int = 15
    max_progress_vars: int = 4
    max_outer_iterations: int = 50000
    stagnation_limit: int = 45
    horizon: int = 32
    population: int = 384
    cem_generations: int = 5
    elite_fraction: float = 0.12


@dataclass
class ArchiveEntry:
    state: Dict[str, Any]
    trajectory: List[List[int]]
    score: float
    target: float


class StateObjective:
    def __init__(self, config: SearchConfig) -> None:
        self.config = config
        self.scalar_vars: List[str] = []
        self.best_high: Dict[str, float] = {}
        self.best_low: Dict[str, float] = {}

    def update_vars(self, new_vars: List[str]):
        self.scalar_vars = new_vars

    def update_records(self, state: Dict[str, Any]) -> None:
        for key in self.scalar_vars + [self.config.target_var]:
            if key in state and is_scalar(state[key]):
                val = float(state[key])
                self.best_high[key] = max(self.best_high.get(key, val), val)
                self.best_low[key] = min(self.best_low.get(key, val), val)

    def state_score(self, state: Dict[str, Any]) -> float:
        score = 1000.0 * state.get(self.config.target_var, 0.0)
        for key in self.scalar_vars:
            val = float(state.get(key, 0.0))
            high = self.best_high.get(key, val)
            low = self.best_low.get(key, val)
            spread = max(1.0, abs(high - low))
            score += 10.0 * (val / spread)
        return float(score)

    def cell(self, state: Dict[str, Any]) -> Tuple[Tuple[str, int], ...]:
        parts = [
            (
                self.config.target_var,
                int(math.floor(float(state.get(self.config.target_var, 0.0)))),
            )
        ]
        for key in self.scalar_vars:
            val = float(state.get(key, 0.0))
            high = self.best_high.get(key, val)
            low = self.best_low.get(key, val)
            width = 1.0 if abs(high - low) <= 16 else (4.0 if abs(high - low) <= 128 else 8.0)
            parts.append((key, int(math.floor(val / width))))
        return tuple(parts)


class StateArchive:
    def __init__(self, objective: StateObjective, rng: random.Random) -> None:
        self.objective = objective
        self.rng = rng
        self.entries_by_cell: Dict[Tuple[Tuple[str, int], ...], ArchiveEntry] = {}
        self.best_entry: Optional[ArchiveEntry] = None
        self.best_target = -math.inf

    def add(self, state: Dict[str, Any], trajectory: List[List[int]]) -> bool:
        self.objective.update_records(state)
        score = self.objective.state_score(state)
        target = float(state.get(self.objective.config.target_var, 0.0))
        cell = self.objective.cell(state)
        existing = self.entries_by_cell.get(cell)

        changed = False
        if existing is None or score > existing.score or len(trajectory) < len(existing.trajectory):
            self.entries_by_cell[cell] = ArchiveEntry(copy.deepcopy(state), copy.deepcopy(trajectory), score, target)
            changed = True

        if target > self.best_target:
            self.best_target = target
            self.best_entry = self.entries_by_cell[cell]
            changed = True

        return changed

    def reindex(self) -> None:
        old_entries = list(self.entries_by_cell.values())
        self.entries_by_cell.clear()
        for entry in old_entries:
            self.add(entry.state, entry.trajectory)

    def select(self) -> ArchiveEntry:
        entries = sorted(self.entries_by_cell.values(), key=lambda e: (e.target, e.score), reverse=True)
        candidates = entries[: max(1, int(len(entries) * 0.15))]
        weights = [max(1e-6, e.score + 1.0) for e in candidates]
        return random.choices(candidates, weights=weights, k=1)[0]


class MLGoExploreCEM:
    def __init__(self, plc: RustPipelinePLC, config: SearchConfig) -> None:
        self.plc = plc
        self.config = config
        self.rng = random.Random(42)
        self.objective = StateObjective(config)
        self.archive = StateArchive(self.objective, self.rng)
        self.discoverer = ProgressVariableDiscoverer(config.target_var, config.max_progress_vars)
        self.attributor = InputAttributor(config.target_var)
        self.stagnation_counter = 0
        self.last_best_target = -math.inf

    def run(self) -> None:
        self.plc.reset_state()
        self.archive.add(self.plc.get_state(), [])
        print(f"Starting Intelligent Search for Target: {self.config.target_var}")

        for iteration in range(1, self.config.max_outer_iterations + 1):
            parent = self.archive.select()
            results = self.run_cem(parent)

            for final_state, states, actions in results:
                target = float(final_state.get(self.config.target_var, 0.0))
                self.discoverer.add_rollout(states, target)

                traj = copy.deepcopy(parent.trajectory)
                for a, s in zip(actions, states):
                    traj.append(a)
                    self.archive.add(s, traj)

            if self.archive.best_target > self.last_best_target:
                self.last_best_target = self.archive.best_target
                self.stagnation_counter = 0
            else:
                self.stagnation_counter += 1

            if iteration % self.config.discovery_interval == 0:
                old_vars = self.objective.scalar_vars
                force_random = self.stagnation_counter >= self.config.stagnation_limit
                new_vars = self.discoverer.analyze(old_vars, force_random=force_random)

                vars_changed = set(new_vars) != set(old_vars)
                if vars_changed:
                    print(f"\n[Variable Update] Tracking new parameters: {new_vars}")
                    self.objective.update_vars(new_vars)
                    self.archive.reindex()
                    print(f"Archive re-indexed. Unique states mapped: {len(self.archive.entries_by_cell)}\n")
                    if force_random:
                        self.stagnation_counter = 0
                elif force_random:
                    print("  -> [STAGNATION DETECTED] Variable set unchanged; retrying forced selection next discovery interval.")

            if iteration % 5 == 0:
                print(
                    f"Iter {iteration:03d} | Best: {self.archive.best_target} | "
                    f"Cells: {len(self.archive.entries_by_cell)} | Stagnation: {self.stagnation_counter}"
                )

            # Periodically log the current regime's attribution map.
            if iteration % 25 == 0:
                self.attributor.log_summary(parent.state)

            if self.archive.best_target >= self.config.target_goal:
                print(f"\nGoal Reached! Optimal sequence length: {len(self.archive.best_entry.trajectory)}")
                return

    def run_cem(self, parent: ArchiveEntry) -> List[Tuple]:
        # Build the reference action sequence used for attribution.
        # Default: midpoint of action ranges. Override with parent's last action if available.
        ref_action = np.full(
            (self.config.horizon, len(ACTION_LOW)), 127, dtype=np.uint8
        )
        ref_action[:, 6] = 0  # cmd byte off by default

        mean = [[127.5] * ANALOG_DIMS for _ in range(self.config.horizon)]
        std  = [[ATTR_STD_MAX] * ANALOG_DIMS for _ in range(self.config.horizon)]

        if parent.trajectory:
            last_a = parent.trajectory[-1]
            last_arr = np.array(last_a, dtype=np.uint8)[: len(ACTION_LOW)]
            for t in range(self.config.horizon):
                ref_action[t, :] = last_arr
                mean[t] = [float(x) for x in last_a[:ANALOG_DIMS]]

        # IIAM: get attribution-weighted per-byte std for this regime.
        # This replaces the flat 90.0 baseline with narrower std for bytes that
        # causally affect the target and wider std for bytes that don't.
        attr_std = self.attributor.get_or_compute(
            self.plc, parent, ref_action, self.config.horizon
        )
        for t in range(self.config.horizon):
            for dim in range(ANALOG_DIMS):
                std[t][dim] = float(attr_std[dim])

        best_rollouts = []
        for _ in range(self.config.cem_generations):
            seq_np = np.zeros((self.config.population, self.config.horizon, len(ACTION_LOW)), dtype=np.uint8)
            seq_list: List[List[List[int]]] = []
            for rollout_idx in range(self.config.population):
                seq: List[List[int]] = []
                for t in range(self.config.horizon):
                    analog = [int(self.rng.gauss(mean[t][i], std[t][i])) for i in range(ANALOG_DIMS)]
                    cmd = 1 if self.rng.random() < 0.5 else 0
                    step_vals = [max(l, min(h, v)) for v, l, h in zip(analog + [cmd], ACTION_LOW, ACTION_HIGH)]
                    seq.append(step_vals)
                    seq_np[rollout_idx, t, :] = np.array(step_vals, dtype=np.uint8)
                seq_list.append(seq)

            raw_parent = parent.state.get("_raw_state")
            if not isinstance(raw_parent, (bytes, bytearray)):
                self.plc.set_state(parent.state)
                raw_parent = self.plc.session.state()

            raw_states = self.plc.rollout_states_batch(bytes(raw_parent), seq_np)

            # Decode all (population × horizon) states in one Rust call — no per-cell FFI round-trips.
            decoded_flat = self.plc.decode_raw_states_batch(raw_states)

            rollouts = []
            for rollout_idx in range(self.config.population):
                states: List[Dict[str, Any]] = []
                best_score = -math.inf
                for step_idx in range(self.config.horizon):
                    state = decoded_flat[rollout_idx * self.config.horizon + step_idx]
                    states.append(state)
                    best_score = max(best_score, self.objective.state_score(state))
                    if int(state.get("fill_head", 0)) >= 64:
                        break

                rollouts.append((states[-1], states, seq_list[rollout_idx], best_score))

            rollouts.sort(key=lambda x: x[3], reverse=True)
            elites = rollouts[: int(self.config.population * self.config.elite_fraction)]
            best_rollouts.extend(elites[:5])

            for t in range(self.config.horizon):
                for dim in range(ANALOG_DIMS):
                    vals = [e[2][t][dim] for e in elites if t < len(e[2])]
                    if vals:
                        elite_mean = sum(vals) / len(vals)
                        elite_std  = max(4.0, statistics.stdev(vals) if len(vals) > 1 else 4.0)
                        mean[t][dim] = elite_mean
                        # Allow CEM to narrow std via elite convergence, but not widen
                        # beyond the attribution ceiling for this byte. Bytes the
                        # attribution identified as irrelevant stay wide; relevant bytes
                        # are allowed to converge further if elites agree.
                        std[t][dim] = min(elite_std, float(attr_std[dim]))

        return [(r[0], r[1], r[2]) for r in best_rollouts]


def is_scalar(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def main() -> None:
    _ = argparse.ArgumentParser()
    config = SearchConfig()
    plc = RustPipelinePLC()
    optimizer = MLGoExploreCEM(plc, config)
    optimizer.run()


if __name__ == "__main__":
    main()
