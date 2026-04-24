use core::num::NonZeroUsize;
use std::mem::size_of;

use libafl::{
    corpus::{InMemoryCorpus, OnDiskCorpus},
    events::SimpleEventManager,
    executors::inprocess::InProcessExecutor,
    feedbacks::{MaxMapFeedback, CrashFeedback},
    fuzzer::{Fuzzer, StdFuzzer},
    generators::RandBytesGenerator,
    inputs::{BytesInput, HasTargetBytes},
    monitors::SimpleMonitor,
    mutators::{havoc_mutations::havoc_mutations, scheduled::HavocScheduledMutator},
    // In 0.15, the Power Scheduler logic is handled by the WeightedScheduler
    schedulers::{StdWeightedScheduler, powersched::PowerSchedule},
    stages::StdMutationalStage,
    state::{StdState, HasCorpus},
    feedback_or,
};
// Direct imports instead of gated prelude
use libafl_bolts::{AsSlice, rands::StdRand, tuples::tuple_list};
use libafl_targets::std_edges_map_observer;
use libafl::observers::StdMapObserver;

use crate::common::{
    harness_boot_plc, harness_reset_plc, harness_fuzz_time_series, 
    plc_get_input_size, plc_get_full_state
};

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

static mut PLC_METRICS: [u8; 4] = [0; 4];

pub fn run() {
    println!("🚀 Starting State-Maximization Greybox Fuzzer...");

    let edges_observer = unsafe { std_edges_map_observer("edges") };
    
    // Safety: Convert static mut to slice using Edition 2024 raw pointers
    let metrics_slice = unsafe { 
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(PLC_METRICS) as *mut u8, 4) 
    };
    let metrics_observer = unsafe{StdMapObserver::new("metrics", metrics_slice)};

    let mut feedback = feedback_or!(
        MaxMapFeedback::new(&edges_observer),
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

    // 0.15 FIX: Use StdWeightedScheduler with PowerSchedule::Fast
    let scheduler = StdWeightedScheduler::with_schedule(
        &mut state, 
        &edges_observer, 
        Some(PowerSchedule::fast())
    );

    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mutator = HavocScheduledMutator::new(havoc_mutations());
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    let monitor = SimpleMonitor::new(|s| println!("{}", s));
    let mut mgr = SimpleEventManager::new(monitor);

    unsafe { harness_boot_plc(); }
    let input_size = unsafe { plc_get_input_size() };

    let mut harness = |input: &BytesInput| {
        let target = input.target_bytes();
        let buf = target.as_slice();
        unsafe {
            harness_reset_plc();
            harness_fuzz_time_series(buf.as_ptr(), buf.len(), input_size);

            let mut current_state = PipelineState::default();
            plc_get_full_state(&mut current_state as *mut _ as *mut u8, size_of::<PipelineState>());

            let metrics_ptr = core::ptr::addr_of_mut!(PLC_METRICS);
            (*metrics_ptr)[0] = current_state.phase as u8;
            (*metrics_ptr)[1] = current_state.prime_score.max(0) as u8;
            (*metrics_ptr)[2] = current_state.flux_score.max(0) as u8;
            (*metrics_ptr)[3] = current_state.fill_head.max(0) as u8;
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

    let mut generator = RandBytesGenerator::new(NonZeroUsize::new(2048).unwrap());
    state.generate_initial_inputs(&mut fuzzer, &mut executor, &mut generator, &mut mgr, 1).unwrap();

    fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr).unwrap();
}