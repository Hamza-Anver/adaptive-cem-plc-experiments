"""Python bindings for libafl_sandbox."""

from typing import TypeAlias

from .libafl_sandbox import PlcVarMeta, PlcVarType, TargetSession, input_size

__version__ = "0.1.0"

VarValue: TypeAlias = int | bool | float

__all__ = [
    "PlcVarMeta",
    "PlcVarType",
    "TargetSession",
    "VarValue",
    "input_size",
]

