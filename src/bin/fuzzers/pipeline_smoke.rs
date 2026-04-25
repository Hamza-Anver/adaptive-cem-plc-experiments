use libafl_sandbox::common::{boot_plc, reset_plc, step_time_series};

fn append_ticks(
    buf: &mut Vec<u8>,
    count: usize,
    pump: u8,
    valve: u8,
    temp: u8,
    bp: u8,
    conc: u8,
    cool: u8,
    cmd: u8,
) {
    for _ in 0..count {
        buf.push(pump);
        buf.push(valve);
        buf.push(temp);
        buf.push(bp);
        buf.push(conc);
        buf.push(cool);
        buf.push(cmd);
    }
}

pub fn run() {
    println!("Running Tick-Perfect Pipeline OOB Smoke Test...");
    
    let mut seq = Vec::new();

    // 1. Idle -> Prime 
    // Cycle 1
    append_ticks(&mut seq, 1, 0, 0, 0, 0, 0, 0, 1);

    // 2. Prime -> Flow
    // Cycles 2 through 25 (Exactly 24 ticks for prime_score to hit 8)
    append_ticks(&mut seq, 24, 60, 30, 0, 60, 0, 0, 0);

    // 3. Flow -> Fill
    // Cycles 26 through 37 (Exactly 12 ticks for flux_score to hit 8, factoring in /2 at cycle 33)
    append_ticks(&mut seq, 12, 60, 30, 60, 60, 40, 60, 0);

    // 4. Fill - Zone 0
    // Starts at Cycle 38 (phase_counter = 1)
    // Needs exactly 35 ticks. At phase_counter = 36, fill_head hits 8.
    append_ticks(&mut seq, 35, 60, 20, 60, 50, 0, 0, 0);

    // 5. Fill - Zone 1
    // Starts at phase_counter = 36. fill_head increments to 8 on the very first tick.
    append_ticks(&mut seq, 32, 60, 30, 70, 50, 0, 0, 0);

    // 6. Fill - Zone 2 (Hits 16 on tick 1)
    append_ticks(&mut seq, 32, 60, 20, 80, 50, 0, 0, 0);

    // 7. Fill - Zone 3 (Hits 24 on tick 1)
    append_ticks(&mut seq, 32, 60, 15, 70, 50, 0, 0, 0);

    // 8. Fill - Zone 4 (Hits 32 on tick 1)
    append_ticks(&mut seq, 32, 80, 25, 60, 50, 0, 0, 0);

    // 9. Fill - Zone 5 (Hits 40 on tick 1)
    append_ticks(&mut seq, 32, 60, 20, 85, 50, 0, 0, 0);

    // 10. Fill - Zone 6 (Hits 48 on tick 1)
    append_ticks(&mut seq, 32, 80, 20, 60, 50, 0, 0, 0);

    // 11. Fill - Zone 7 (Hits 56 on tick 1)
    // 35 ticks ensures fill_head hits 64 on the 32nd tick, plus a few buffer ticks to trigger the abort.
    append_ticks(&mut seq, 35, 70, 20, 70, 50, 0, 0, 0);

    println!("Sequence built. Total ticks: {}, Total bytes: {}", seq.len() / 7, seq.len());
    println!("Executing payload...");

    boot_plc();
    reset_plc();
    step_time_series(&seq, 7);

    println!("Sequence completed. If you see this, the target did NOT crash.");
}
