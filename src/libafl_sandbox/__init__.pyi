from typing import TypeAlias

import numpy as np
import numpy.typing as npt

VarValue: TypeAlias = int | bool | float
"""Supported Python value types for PLC variables."""

class PlcVarType:
    """Enum-like variable type constants from the target metadata."""

    UINT8: int
    UINT16: int
    UINT32: int
    BOOL: int
    FLOAT: int

class PlcVarMeta:
    """Metadata entry describing one PLC variable in full state."""

    name: str
    """Variable name exposed by the target."""

    var_type: PlcVarType
    """Variable type constant."""

    size: int
    """Variable size in bytes."""

    offset: int
    """Byte offset of the variable inside full_state()."""

    def __init__(self, name: str, var_type: PlcVarType, size: int, offset: int) -> None:
        """Create a metadata object.

        Args:
            name: Variable name.
            var_type: Variable type enum value.
            size: Size in bytes.
            offset: Byte offset in full state.
        """

class TargetSession:
    """Session handle for controlling and introspecting the PLC target."""

    def __init__(self) -> None:
        """Create a new session. Call boot() before reset()."""

    def boot(self) -> None:
        """Initialize the target runtime once."""

    def reset(self) -> None:
        """Reset target state to its initial values."""

    def input_size(self) -> int:
        """Return required input bytes per step."""

    def step(self, data: bytes) -> None:
        """Run one target step.

        Args:
            data: Input bytes for a single step.
        """

    def step_series(self, data: bytes, bytes_per_step: int) -> None:
        """Run multiple steps from a concatenated time-series buffer.

        Args:
            data: Concatenated input bytes.
            bytes_per_step: Number of bytes consumed per step.
        """

    def state_size(self) -> int:
        """Return total state size in bytes."""

    def state(self) -> bytes:
        """Return full raw state bytes."""

    def set_state(self, state: bytes) -> bool:
        """Set full raw state bytes.

        Args:
            state: Raw state buffer matching state_size().

        Returns:
            True on success, otherwise False.
        """

    def var_metadata(self) -> list[PlcVarMeta]:
        """Return metadata for all exposed variables."""

    def input_hints(self) -> list[PlcVarMeta]:
        """Return metadata for expected input fields per step."""

    def read_vars(self, names: list[str] | None = None) -> dict[str, VarValue]:
        """Return variables as a dict.

        Args:
            names: Variable names to fetch. If None, returns all variables.

        Returns:
            Mapping of variable name to typed value (int, bool, or float).
        """

    def write_vars(self, values: dict[str, VarValue]) -> None:
        """Set variables from a dict.

        Args:
            values: Mapping of variable name to new value.

        Raises:
            Exception: If a variable name is unknown or a value type/range is invalid.
        """

    def rollout_states_batch(
        self,
        initial_states: npt.NDArray[np.uint8],
        inputs: npt.NDArray[np.uint8],
    ) -> npt.NDArray[np.uint8]:
        """Run batched rollouts and return raw state bytes at each step.

        Args:
            initial_states: Array shaped ``(rollouts, state_size)``.
            inputs: Array shaped ``(rollouts, steps, input_size)``.

        Returns:
            Array shaped ``(rollouts, steps, state_size)`` containing raw state snapshots.

        Raises:
            Exception: If shapes do not match session ``state_size``/``input_size``.
        """

def input_size() -> int:
    """Return required input bytes per step."""
