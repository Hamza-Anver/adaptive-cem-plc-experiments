pub mod common;
pub mod cem_rollout;

#[cfg(feature = "python")]
mod py;

// Re-export core types and functions for library users
pub use common::{
    PlcVarMeta, PlcVarType, all_input_hints, all_var_metadata, boot_plc, input_size, reset_plc, set_state,
    state, state_size, step, step_series, var_types, var_values,
    write_var_values,
};

#[cfg(feature = "python")]
pub use py::{
    libafl_sandbox,
    PlcVarMeta as PythonBindingVarMeta,
    PlcVarType as PythonBindingVarType,
    TargetSession as PythonBindingTargetSession,
};
