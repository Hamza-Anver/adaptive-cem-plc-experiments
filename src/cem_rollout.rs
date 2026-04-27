use rayon::prelude::*;

use crate::common;

pub fn rollout_states_batch(
    initial_states_flat: &[u8],
    num_rollouts: usize,
    state_size: usize,
    inputs_flat: &[u8],
    num_steps: usize,
    input_size: usize,
) -> Result<Vec<u8>, String> {
    if state_size == 0 {
        return Err("state_size must be > 0".to_string());
    }
    if input_size == 0 {
        return Err("input_size must be > 0".to_string());
    }

    let expected_states_len = num_rollouts
        .checked_mul(state_size)
        .ok_or_else(|| "initial_states size overflow".to_string())?;
    if initial_states_flat.len() != expected_states_len {
        return Err(format!(
            "initial_states has invalid length: got {}, expected {} (rollouts={}, state_size={})",
            initial_states_flat.len(),
            expected_states_len,
            num_rollouts,
            state_size
        ));
    }

    let steps_total = num_rollouts
        .checked_mul(num_steps)
        .ok_or_else(|| "inputs shape overflow".to_string())?;
    let expected_inputs_len = steps_total
        .checked_mul(input_size)
        .ok_or_else(|| "inputs size overflow".to_string())?;
    if inputs_flat.len() != expected_inputs_len {
        return Err(format!(
            "inputs has invalid length: got {}, expected {} (rollouts={}, steps={}, input_size={})",
            inputs_flat.len(),
            expected_inputs_len,
            num_rollouts,
            num_steps,
            input_size
        ));
    }

    let output_len = steps_total
        .checked_mul(state_size)
        .ok_or_else(|| "output size overflow".to_string())?;
    let mut out = vec![0u8; output_len];

    // Each rollout writes to a distinct, non-overlapping slice of `out`.
    // The C PLC state is _Thread_local so each Rayon worker thread has its
    // own independent state — no locking needed.
    let rollout_slice = num_steps * state_size;

    out.par_chunks_mut(rollout_slice)
        .enumerate()
        .try_for_each(|(rollout_idx, out_slice)| -> Result<(), String> {
            let init_state =
                &initial_states_flat[rollout_idx * state_size..(rollout_idx + 1) * state_size];

            if !common::set_state(init_state) {
                return Err(format!("set_state failed for rollout {}", rollout_idx));
            }

            for step_idx in 0..num_steps {
                let input_offset = (rollout_idx * num_steps + step_idx) * input_size;
                let input_step = &inputs_flat[input_offset..input_offset + input_size];
                common::step(input_step);

                let step_state = common::state();
                if step_state.len() != state_size {
                    return Err(format!(
                        "state size mismatch at rollout {}, step {}: got {}, expected {}",
                        rollout_idx,
                        step_idx,
                        step_state.len(),
                        state_size
                    ));
                }

                out_slice[step_idx * state_size..(step_idx + 1) * state_size]
                    .copy_from_slice(&step_state);
            }

            Ok(())
        })?;

    Ok(out)
}
