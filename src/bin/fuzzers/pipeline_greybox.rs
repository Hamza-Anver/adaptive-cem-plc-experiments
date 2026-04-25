use core::num::NonZeroUsize;
use std::borrow::Cow;
use std::mem::size_of;

use libafl::{
    corpus::{InMemoryCorpus, OnDiskCorpus},
    events::SimpleEventManager,
    executors::inprocess::InProcessExecutor,
    feedbacks::{MaxMapFeedback, CrashFeedback},
    fuzzer::{Evaluator, Fuzzer, StdFuzzer},
    generators::RandBytesGenerator,
    inputs::{BytesInput, HasMutatorBytes, HasTargetBytes},
    monitors::SimpleMonitor,
    mutators::{MutationResult, Mutator, havoc_mutations::havoc_mutations, scheduled::HavocScheduledMutator},
    // In 0.15, the Power Scheduler logic is handled by the WeightedScheduler
    schedulers::{IndexesLenTimeMinimizerScheduler, StdWeightedScheduler, powersched::PowerSchedule},
    stages::{CalibrationStage, StdPowerMutationalStage},
    state::{HasRand, StdState},
    feedback_or,
};
// Direct imports instead of gated prelude
use libafl_bolts::{AsSlice, Named, rands::Rand, rands::StdRand, tuples::tuple_list};
use libafl_targets::std_edges_map_observer;
use libafl::observers::{CanTrack, StdMapObserver};

use libafl_sandbox::common::{boot_plc, reset_plc, plc_get_full_state, plc_get_input_size, plc_step};

#[repr(C)]
#[derive(Debug)]
pub struct PipelineState {
    pub phase: u32,
    pub cycle_count: i32,
    pub prime_cycles: i32,
    pub prime_score: i32,
    pub flux_score: i32,
    pub flow_accum: i32,
    pub press_accum: i32,
    pub temp_accum: i32,
    pub phase_counter: i32,
    pub fill_head: i32,
    pub buffer: [i32; 64],
    pub pv_sum: i32,
    pub status: i8,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            phase: 0,
            cycle_count: 0,
            prime_cycles: 0,
            prime_score: 0,
            flux_score: 0,
            flow_accum: 0,
            press_accum: 0,
            temp_accum: 0,
            phase_counter: 0,
            fill_head: 0,
            buffer: [0; 64],
            pv_sum: 0,
            status: 0,
        }
    }
}

const TICK_SIZE: usize = 7;
const METRICS_SIZE: usize = 21;

static mut PLC_METRICS: [u8; METRICS_SIZE] = [0; METRICS_SIZE];

#[derive(Debug)]
struct TickGuidedMutator {
    cursor: usize,
}

impl TickGuidedMutator {
    fn new() -> Self {
        Self { cursor: 0 }
    }

    fn snap_value(v: u8) -> u8 {
        const THRESHOLDS: [u8; 15] = [40, 50, 55, 60, 63, 65, 70, 75, 80, 85, 90, 95, 100, 110, 125];
        let mut best = THRESHOLDS[0];
        let mut best_dist = u8::abs_diff(v, best);
        for t in THRESHOLDS.iter().skip(1) {
            let dist = u8::abs_diff(v, *t);
            if dist < best_dist {
                best = *t;
                best_dist = dist;
            }
        }
        best
    }

    fn clamp_cmd_bit(bytes: &mut [u8]) {
        for chunk in bytes.chunks_exact_mut(TICK_SIZE) {
            chunk[6] &= 1;
        }
    }
}

impl Named for TickGuidedMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("TickGuidedMutator");
        &NAME
    }
}

impl<S> Mutator<BytesInput, S> for TickGuidedMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut BytesInput) -> Result<MutationResult, libafl::Error> {
        let bytes = input.mutator_bytes_mut();
        let tick_count = bytes.len() / TICK_SIZE;
        if tick_count == 0 {
            return Ok(MutationResult::Skipped);
        }

        let mode = state.rand_mut().below_or_zero(3);

        if mode == 0 {
            let tick = self.cursor % tick_count;
            self.cursor = self.cursor.wrapping_add(1);
            let base = tick * TICK_SIZE;
            // Deterministic threshold snapping for the six analog channels.
            for ch in 0..6 {
                bytes[base + ch] = Self::snap_value(bytes[base + ch]);
            }
        } else if mode == 1 {
            let channel = state.rand_mut().below_or_zero(6);
            let start_tick = state.rand_mut().below_or_zero(tick_count);
            let max_window = tick_count.saturating_sub(start_tick).max(1);
            let window = 1 + state.rand_mut().below_or_zero(max_window.min(24));
            let value = Self::snap_value((state.rand_mut().below_or_zero(256)) as u8);
            for t in start_tick..(start_tick + window).min(tick_count) {
                bytes[t * TICK_SIZE + channel] = value;
            }
        } else {
            // Tick-level replacement using a small set of phase-valid templates.
            const TEMPLATES: [[u8; TICK_SIZE]; 4] = [
                [0, 0, 0, 0, 0, 0, 1],
                [60, 30, 0, 60, 0, 0, 0],
                [60, 30, 60, 60, 40, 60, 0],
                [70, 20, 70, 50, 0, 0, 0],
            ];
            let tick = state.rand_mut().below_or_zero(tick_count);
            let template = state.rand_mut().below_or_zero(TEMPLATES.len());
            let base = tick * TICK_SIZE;
            bytes[base..base + TICK_SIZE].copy_from_slice(&TEMPLATES[template]);
        }

        Self::clamp_cmd_bit(bytes);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<libafl::corpus::CorpusId>) -> Result<(), libafl::Error> {
        Ok(())
    }
}

#[derive(Debug)]
struct SuffixMutator;

impl SuffixMutator {
    fn new() -> Self {
        Self
    }
}

impl Named for SuffixMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("SuffixMutator");
        &NAME
    }
}

impl<S> Mutator<BytesInput, S> for SuffixMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut BytesInput) -> Result<MutationResult, libafl::Error> {
        let bytes = input.mutator_bytes_mut();
        let aligned_len = bytes.len() - (bytes.len() % TICK_SIZE);
        let tick_count = aligned_len / TICK_SIZE;
        if tick_count == 0 {
            return Ok(MutationResult::Skipped);
        }

        // Bias split heavily toward the end: keep 70-90% prefix unchanged.
        let keep_percent = 70 + state.rand_mut().below_or_zero(21); // [70, 90]
        let keep_ticks = ((tick_count * keep_percent as usize) / 100).min(tick_count.saturating_sub(1));
        let split_idx = keep_ticks * TICK_SIZE;

        let suffix = &mut bytes[split_idx..aligned_len];
        if suffix.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        // Basic havoc operations on suffix only.
        let op_count = 1 + state.rand_mut().below_or_zero(8);
        for _ in 0..op_count {
            let idx = state.rand_mut().below_or_zero(suffix.len());
            match state.rand_mut().below_or_zero(3) {
                0 => {
                    // Bit flip
                    let bit = 1u8 << state.rand_mut().below_or_zero(8);
                    suffix[idx] ^= bit;
                }
                1 => {
                    // Byte replacement
                    suffix[idx] = state.rand_mut().below_or_zero(256) as u8;
                }
                _ => {
                    // Small arithmetic mutation
                    let delta = 1 + state.rand_mut().below_or_zero(16) as u8;
                    if state.rand_mut().below_or_zero(2) == 0 {
                        suffix[idx] = suffix[idx].wrapping_add(delta);
                    } else {
                        suffix[idx] = suffix[idx].wrapping_sub(delta);
                    }
                }
            }
        }

        TickGuidedMutator::clamp_cmd_bit(suffix);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<libafl::corpus::CorpusId>) -> Result<(), libafl::Error> {
        Ok(())
    }
}

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

fn bootstrap_phase_seeds() -> Vec<BytesInput> {
    let mut seeds = Vec::new();

    // Seed 1: idle -> prime transition only.
    let mut s1 = Vec::new();
    append_ticks(&mut s1, 1, 0, 0, 0, 0, 0, 0, 1);
    seeds.push(BytesInput::new(s1));

    // Seed 2: stable prime progression toward flow transition.
    let mut s2 = Vec::new();
    append_ticks(&mut s2, 1, 0, 0, 0, 0, 0, 0, 1);
    append_ticks(&mut s2, 24, 60, 30, 0, 60, 0, 0, 0);
    seeds.push(BytesInput::new(s2));

    // Seed 3: reaches fill entry and starts accumulating fill progress.
    let mut s3 = Vec::new();
    append_ticks(&mut s3, 1, 0, 0, 0, 0, 0, 0, 1);
    append_ticks(&mut s3, 24, 60, 30, 0, 60, 0, 0, 0);
    append_ticks(&mut s3, 12, 60, 30, 60, 60, 40, 60, 0);
    append_ticks(&mut s3, 16, 60, 20, 60, 50, 0, 0, 0);
    seeds.push(BytesInput::new(s3));

    seeds
}

fn bucketize_nonneg(v: i32, step: i32, cap: u8) -> u8 {
    if v <= 0 {
        return 0;
    }
    let b = (v / step) as i64;
    b.min(cap as i64) as u8
}

fn zone_bounds(fill_head: i32) -> (i32, i32, i32, i32) {
    if fill_head < 8 {
        (60, 90, 50, 65)
    } else if fill_head < 16 {
        (80, 110, 62, 77)
    } else if fill_head < 24 {
        (70, 100, 72, 87)
    } else if fill_head < 32 {
        (55, 85, 65, 80)
    } else if fill_head < 40 {
        (95, 125, 55, 70)
    } else if fill_head < 48 {
        (65, 95, 78, 93)
    } else if fill_head < 56 {
        (85, 115, 52, 67)
    } else {
        (75, 105, 63, 78)
    }
}

fn dist_to_range(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0
    }
}

fn zone_validity_score(fill_head: i32, current_pv: i32, temp: i32) -> u8 {
    let (pv_lo, pv_hi, t_lo, t_hi) = zone_bounds(fill_head);
    let pv_dist = dist_to_range(current_pv, pv_lo, pv_hi);
    let t_dist = dist_to_range(temp, t_lo, t_hi);

    if pv_dist == 0 && t_dist == 0 {
        let pv_margin = (current_pv - pv_lo).min(pv_hi - current_pv);
        let t_margin = (temp - t_lo).min(t_hi - temp);
        let margin = pv_margin.min(t_margin).max(0);
        (200 + (margin.min(11) * 5)) as u8
    } else {
        let penalty = (pv_dist + t_dist).min(199);
        (199 - penalty) as u8
    }
}

pub fn run() {
    println!("🚀 Starting State-Maximization Greybox Fuzzer...");

    // Track map indices to enable minimizer scheduling.
    let edges_observer = unsafe { std_edges_map_observer("edges") }.track_indices();
    
    // Safety: Convert static mut to slice using Edition 2024 raw pointers
    let metrics_slice = unsafe { 
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(PLC_METRICS) as *mut u8, METRICS_SIZE)
    };
    let metrics_observer = unsafe{StdMapObserver::new("metrics", metrics_slice)};

    let edges_feedback = MaxMapFeedback::new(&edges_observer);
    let calibration = CalibrationStage::new(&edges_feedback);

    let mut feedback = feedback_or!(
        edges_feedback,
        MaxMapFeedback::new(&metrics_observer)
    );
    let mut objective = CrashFeedback::new();

    let mut state = StdState::new(
        StdRand::with_seed(0),
        InMemoryCorpus::new(),
        OnDiskCorpus::new("crashes").unwrap(),
        &mut feedback,
        &mut objective,
    ).unwrap();

    // Phase 1: weighted power scheduling wrapped by a corpus minimizer.
    let base_scheduler = StdWeightedScheduler::with_schedule(
        &mut state, 
        &edges_observer, 
        Some(PowerSchedule::fast())
    );
    let scheduler = IndexesLenTimeMinimizerScheduler::new(&edges_observer, base_scheduler);

    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let guided_mutator = TickGuidedMutator::new();
    let suffix_mutator = SuffixMutator::new();
    let havoc_mutator = HavocScheduledMutator::new(havoc_mutations());
    let mut stages = tuple_list!(
        calibration,
        StdPowerMutationalStage::new(guided_mutator),
        StdPowerMutationalStage::new(suffix_mutator),
        StdPowerMutationalStage::new(havoc_mutator)
    );

    let monitor = SimpleMonitor::new(|s| println!("{}", s));
    let mut mgr = SimpleEventManager::new(monitor);

    boot_plc();
    let input_size = unsafe { plc_get_input_size() };

    let mut harness = |input: &BytesInput| {
        let target = input.target_bytes();
        let buf = target.as_slice();
        unsafe {
            reset_plc();

            let metrics_ptr = core::ptr::addr_of_mut!(PLC_METRICS);
            (*metrics_ptr).fill(0);

            // Keep execution aligned to whole PLC ticks.
            let aligned_len = buf.len() - (buf.len() % input_size);
            let mut current_state = PipelineState::default();
            let mut max_fill_head = 0i32;
            let mut best_bucket: i32 = -1;
            let mut best_zone_score = 0u8;

            for tick in buf[..aligned_len].chunks_exact(TICK_SIZE) {
                plc_step(tick.as_ptr(), TICK_SIZE);

                plc_get_full_state(
                    &mut current_state as *mut _ as *mut u8,
                    size_of::<PipelineState>(),
                );

                max_fill_head = max_fill_head.max(current_state.fill_head.max(0));

                let current_pv = (tick[0] as i32) + (tick[1] as i32);
                let pipe_temp = tick[2] as i32;
                let zone_score = zone_validity_score(current_state.fill_head, current_pv, pipe_temp);
                let bucket = (current_state.fill_head.max(0) / 8).min(7);

                if bucket > best_bucket {
                    best_bucket = bucket;
                    best_zone_score = zone_score;
                } else if bucket == best_bucket {
                    best_zone_score = best_zone_score.max(zone_score);
                }
            }

            let flow_ready = (current_state.flow_accum > 6) as u8;
            let press_ready = (current_state.press_accum > 5) as u8;
            let temp_ready = (current_state.temp_accum > 6) as u8;

            (*metrics_ptr)[0] = current_state.phase.min(3) as u8;
            (*metrics_ptr)[1] = bucketize_nonneg(current_state.prime_score, 2, 15);
            (*metrics_ptr)[2] = bucketize_nonneg(current_state.flux_score, 2, 15);
            (*metrics_ptr)[3] = max_fill_head.min(255) as u8;
            (*metrics_ptr)[4] = flow_ready;
            (*metrics_ptr)[5] = press_ready;
            (*metrics_ptr)[6] = temp_ready;
            (*metrics_ptr)[7] = (flow_ready == 1 && press_ready == 1 && temp_ready == 1) as u8;
            (*metrics_ptr)[8] = current_state.fill_head.max(0).rem_euclid(8) as u8;
            (*metrics_ptr)[9] = bucketize_nonneg(current_state.phase_counter, 8, 31);
            (*metrics_ptr)[10] = bucketize_nonneg(current_state.cycle_count, 16, 255);
            (*metrics_ptr)[11] = bucketize_nonneg(current_state.pv_sum, 8, 31);
            (*metrics_ptr)[12] = if best_bucket >= 0 { (best_bucket as u8) + 1 } else { 0 };

            if best_bucket >= 0 {
                let zone_metric_idx = 13 + (best_bucket as usize);
                (*metrics_ptr)[zone_metric_idx] = best_zone_score;
            }
        }
        libafl::executors::ExitKind::Ok
    };

    let mut executor = InProcessExecutor::new(
        &mut harness,
        tuple_list!(edges_observer, metrics_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
    ).unwrap();

    for seed in bootstrap_phase_seeds() {
        fuzzer.add_input(&mut state, &mut executor, &mut mgr, seed).unwrap();
    }

    // Phase 2: use tick-aligned random seeds at multiple sequence scales.
    let mut short_gen = RandBytesGenerator::new(NonZeroUsize::new(input_size * 64).unwrap());
    state.generate_initial_inputs(&mut fuzzer, &mut executor, &mut short_gen, &mut mgr, 2).unwrap();

    let mut medium_gen = RandBytesGenerator::new(NonZeroUsize::new(input_size * 256).unwrap());
    state.generate_initial_inputs(&mut fuzzer, &mut executor, &mut medium_gen, &mut mgr, 2).unwrap();

    let mut long_gen = RandBytesGenerator::new(NonZeroUsize::new(input_size * 900).unwrap());
    state.generate_initial_inputs(&mut fuzzer, &mut executor, &mut long_gen, &mut mgr, 2).unwrap();

    fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr).unwrap();
}
