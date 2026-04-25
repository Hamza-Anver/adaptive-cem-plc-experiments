pub mod common;

#[cfg(feature = "python")]
mod py;

// Re-export core types and functions for library users
pub use common::{
    PlcVarMeta, PlcVarType, boot_plc, reset_plc, input_size, step, step_time_series,
    full_state_size, full_state, set_full_state, get_all_var_metadata,
};

#[cfg(feature = "python")]
pub use py::{libafl_sandbox, PyTargetSession, PyPlcVarMeta, PyPlcVarType};
