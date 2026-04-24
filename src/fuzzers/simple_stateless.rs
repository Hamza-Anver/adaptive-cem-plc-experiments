use core::num::NonZeroUsize;

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
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::StdState,
};
use libafl_bolts::{AsSlice, rands::StdRand, tuples::tuple_list};
use libafl_targets::std_edges_map_observer;

// Import our C bindings from common.rs
use crate::common::{harness_boot_plc, harness_reset_plc, harness_fuzz_one_tick};

pub fn run() {
    println!("Starting Simple Stateless Fuzzer...");

    let edges_observer = unsafe { std_edges_map_observer("edges") };

    let mut feedback = MaxMapFeedback::new(&edges_observer);
    let mut objective = CrashFeedback::new();

    let mut state = StdState::new(
        StdRand::with_seed(0),
        InMemoryCorpus::new(),
        OnDiskCorpus::new("crashes").unwrap(),
        &mut feedback,
        &mut objective,
    )
    .unwrap();

    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mutator = HavocScheduledMutator::new(havoc_mutations());
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    let monitor = SimpleMonitor::new(|s| println!("{}", s));
    let mut mgr = SimpleEventManager::new(monitor);

    // Boot the PLC hardware once
    unsafe {
        harness_boot_plc();
    }

    // The execution harness
    let mut harness = |input: &BytesInput| {
        let target = input.target_bytes();
        let buf = target.as_slice();
        unsafe {
            harness_reset_plc();
            harness_fuzz_one_tick(buf.as_ptr(), buf.len());
        }
        libafl::executors::ExitKind::Ok
    };

    let mut executor = InProcessExecutor::new(
        &mut harness,
        tuple_list!(edges_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
    )
    .unwrap();

    // Generate initial seeds
    let mut generator = RandBytesGenerator::new(NonZeroUsize::new(64).unwrap());
    state
        .generate_initial_inputs(&mut fuzzer, &mut executor, &mut generator, &mut mgr, 1)
        .unwrap();

    // Start the main fuzzing loop
    fuzzer
        .fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr)
        .unwrap();
}