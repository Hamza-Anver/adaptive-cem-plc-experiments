#!/usr/bin/env python3
"""
Hybrid Go-Explore + CEM with ML Deduction, Heuristic Pruning, and Stagnation Breaking.
"""

from __future__ import annotations

import argparse
import copy
import math
import random
import statistics
from collections import deque
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np
import pandas as pd
from sklearn.cluster import KMeans
from sklearn.ensemble import RandomForestRegressor
from sklearn.preprocessing import StandardScaler

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
        """Decode a (N, state_size) uint8 array into N dicts without touching global PLC state."""
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


class ProgressVariableDiscoverer:
    def __init__(self, target_var: str, max_vars: int = 5, n_regimes: int = 5, hints: List[str] = None):
        self.target_var = target_var
        self.max_vars = max_vars
        self.n_regimes = n_regimes
        # User-specified variables that are always included regardless of ML output.
        # They bypass heuristic filters and count against max_vars.
        self.hints: List[str] = [v for v in (hints or []) if v != target_var]
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

    def _apply_hints(self, discovered: List[str]) -> List[str]:
        """Prepend pinned hint variables, fill remaining slots from discovered."""
        result = list(self.hints)
        remaining = self.max_vars - len(result)
        for v in discovered:
            if v not in result and remaining > 0:
                result.append(v)
                remaining -= 1
        return result

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
            # Hints are always kept; random injection fills remaining slots.
            slots = self.max_vars - len(self.hints)
            current_set = set(current_vars) | set(self.hints)
            preferred = [col for col in valid_cols if col not in current_set]

            if not preferred and not self.hints:
                print("  -> [STAGNATION DETECTED] No alternative variables available; keeping current set.")
                return current_vars

            chosen: List[str] = []
            if len(preferred) >= slots:
                chosen = random.sample(preferred, slots)
            else:
                chosen = preferred[:]
                needed = slots - len(chosen)
                fallback_pool = [col for col in valid_cols if col not in chosen and col not in self.hints]
                if needed > 0 and fallback_pool:
                    chosen.extend(random.sample(fallback_pool, min(needed, len(fallback_pool))))
            return self._apply_hints(chosen)

        y = np.array(self.target_buffer)
        if np.std(y) < 1e-5:
            variances = df[valid_cols].var().sort_values(ascending=False)
            top_by_var = variances.index[: self.max_vars].tolist()
            return self._apply_hints(top_by_var)

        X_raw = df[valid_cols]

        # Regime detection: include fill_head in clustering so clusters align with
        # actual target zones, then train the RF only on the frontier cluster.
        MIN_CLUSTER = 80
        k = min(self.n_regimes, len(X_raw) // MIN_CLUSTER)
        X_fit, y_fit = X_raw, y  # default: use all data

        if k >= 2:
            # Cluster on valid_cols + target so zones are distinguished by fill_head level.
            cluster_cols = valid_cols + [self.target_var]
            X_cluster = df[cluster_cols]
            scaler = StandardScaler()
            X_scaled = scaler.fit_transform(X_cluster)

            km = KMeans(n_clusters=k, n_init=5, random_state=42)
            labels = km.fit_predict(X_scaled)

            # Frontier = cluster with the highest mean fill_head among its member states.
            cluster_fill_head_means = {
                c: df[self.target_var].values[labels == c].mean() for c in range(k)
            }
            frontier = max(cluster_fill_head_means, key=cluster_fill_head_means.get)
            mask = labels == frontier
            n_frontier = int(mask.sum())

            if n_frontier >= MIN_CLUSTER:
                X_fit, y_fit = X_raw[mask], y[mask]
                print(
                    f"  [regime] k={k}, frontier cluster {frontier}"
                    f" (mean {self.target_var}={cluster_fill_head_means[frontier]:.1f}, n={n_frontier})"
                )
            else:
                print(f"  [regime] frontier cluster too small ({n_frontier} rows), using all data")
        else:
            print(f"  [regime] not enough data for {self.n_regimes} clusters, using all data")

        if self.hints:
            print(f"  [hints] pinned: {self.hints}")

        rf = RandomForestRegressor(n_estimators=50, max_depth=6, random_state=42, n_jobs=-1)
        rf.fit(X_fit, y_fit)

        importances = rf.feature_importances_
        sorted_idx = np.argsort(importances)[::-1]

        best_vars = []
        for idx in sorted_idx:
            if importances[idx] > 0.015:
                best_vars.append(valid_cols[idx])
                if len(best_vars) >= self.max_vars:
                    break

        return self._apply_hints(best_vars if best_vars else current_vars)


class RegimePriorMemory:
    """
    Stores a per-regime CEM input prior learned from elite rollouts.

    After each CEM run the final elite distribution (mean and std per input
    dimension, averaged across timesteps) is folded into the stored prior via
    an EMA.  The next CEM run in the same regime initialises its Gaussian from
    this prior instead of the uninformed flat default.

    Regime key: (phase, fill_zone) where fill_zone = fill_head // 8.
    For phases 0-2, fill_head is always 0 so the zone is always 0.

    This is equivalent to accumulating the gradient of the CEM objective
    (E[R] w.r.t. the sampling distribution) across outer iterations, giving
    the algorithm a warm start that improves over time without any hardcoded
    domain knowledge.
    """

    def __init__(self, n_dims: int, alpha: float = 0.3):
        self.n_dims = n_dims
        self.alpha = alpha  # EMA weight for new observations (higher = faster adaptation)
        self._priors: Dict[Tuple[int, int], Tuple[List[float], List[float]]] = {}

    @staticmethod
    def _key(state: Dict[str, Any]) -> Tuple[int, int]:
        phase = int(state.get("phase", 0))
        fill_head = int(state.get("fill_head", 0))
        zone = fill_head // 8 if phase == 3 else 0
        return (phase, zone)

    def get(self, state: Dict[str, Any]) -> Optional[Tuple[List[float], List[float]]]:
        return self._priors.get(self._key(state))

    def update(self, state: Dict[str, Any], mean: List[float], std: List[float]) -> None:
        key = self._key(state)
        if key not in self._priors:
            self._priors[key] = (list(mean), list(std))
            print(f"  [regime prior] new entry (phase={key[0]}, zone={key[1]}): "
                  f"mean=[{', '.join(f'{v:.0f}' for v in mean)}]")
        else:
            old_mean, old_std = self._priors[key]
            new_mean = [self.alpha * m + (1 - self.alpha) * om for m, om in zip(mean, old_mean)]
            new_std  = [self.alpha * s + (1 - self.alpha) * os for s, os in zip(std,  old_std)]
            self._priors[key] = (new_mean, new_std)


@dataclass
class SearchConfig:
    target_var: str = "fill_head"
    target_goal: float = 64.0
    discovery_interval: int = 15
    max_progress_vars: int = 4
    max_outer_iterations: int = 5000
    stagnation_limit: int = 45
    horizon: int = 128
    population: int = 256
    cem_generations: int = 5
    elite_fraction: float = 0.12
    n_regimes: int = 5  # KMeans clusters for regime-aware variable selection
    # EMA weight for updating the per-regime CEM prior (0 = never update, 1 = replace each time).
    regime_prior_alpha: float = 0.3
    # Variables always tracked as progress indicators regardless of ML output.
    progress_hints: List[str] = field(default_factory=lambda: ["phase"])


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
        self.discoverer = ProgressVariableDiscoverer(
            config.target_var, config.max_progress_vars, config.n_regimes, config.progress_hints
        )
        self.regime_memory = RegimePriorMemory(ANALOG_DIMS, config.regime_prior_alpha)
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
                    # Keep retrying forced-random at each discovery tick until variables actually change.
                    print("  -> [STAGNATION DETECTED] Variable set unchanged; retrying forced selection next discovery interval.")

            if iteration % 5 == 0:
                print(
                    f"Iter {iteration:03d} | Best: {self.archive.best_target} | "
                    f"Cells: {len(self.archive.entries_by_cell)} | Stagnation: {self.stagnation_counter}"
                )

            if self.archive.best_target >= self.config.target_goal:
                print(f"\nGoal Reached! Optimal sequence length: {len(self.archive.best_entry.trajectory)}")
                return

    def run_cem(self, parent: ArchiveEntry) -> List[Tuple]:
        # Initialise from the learned regime prior if one exists, otherwise
        # fall back to the last trajectory action, otherwise the flat default.
        prior = self.regime_memory.get(parent.state)
        if prior is not None:
            prior_mean, prior_std = prior
            mean = [list(prior_mean) for _ in range(self.config.horizon)]
            std  = [list(prior_std)  for _ in range(self.config.horizon)]
        elif parent.trajectory:
            last_a = parent.trajectory[-1]
            mean = [[float(x) for x in last_a[:ANALOG_DIMS]] for _ in range(self.config.horizon)]
            std  = [[90.0] * ANALOG_DIMS for _ in range(self.config.horizon)]
        else:
            mean = [[127.5] * ANALOG_DIMS for _ in range(self.config.horizon)]
            std  = [[90.0]  * ANALOG_DIMS for _ in range(self.config.horizon)]

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
                        mean[t][dim] = sum(vals) / len(vals)
                        std[t][dim] = max(4.0, statistics.stdev(vals) if len(vals) > 1 else 4.0)

        # Update the regime prior with the final elite distribution aggregated
        # across timesteps.  Using all timesteps gives a broader sample of what
        # inputs work in this regime rather than just the last step.
        if elites:
            agg_mean, agg_std = [], []
            for dim in range(ANALOG_DIMS):
                vals = [e[2][t][dim] for e in elites for t in range(len(e[2]))]
                if vals:
                    agg_mean.append(sum(vals) / len(vals))
                    agg_std.append(max(8.0, statistics.stdev(vals) if len(vals) > 1 else 8.0))
                else:
                    agg_mean.append(mean[0][dim])
                    agg_std.append(std[0][dim])
            self.regime_memory.update(parent.state, agg_mean, agg_std)

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
