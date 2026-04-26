"""Python bindings for libafl_sandbox."""

from typing import TypeAlias

from .libafl_sandbox import PyPlcVarMeta, PyPlcVarType, PyTargetSession, py_input_size

__version__ = "0.1.0"

VarValue: TypeAlias = int | bool | float

__all__ = ["PyPlcVarMeta", "PyPlcVarType", "PyTargetSession", "VarValue", "py_input_size"]

PyTargetSession.get_vars.__doc__ = "Return variable values as a dict."
PyTargetSession.set_vars.__doc__ = "Set variables from a dict."

